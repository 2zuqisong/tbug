import { run as runPty, PtyResult } from '../executor/pty.js';
import { getDefaultClient, type ChatMessage, type ChatStreamEvent } from '../client/llm.js';
import { getToolDefinitions, executeTool, type ToolDefinition } from '../tools/index.js';
import { askUserConfirmation } from '../utils/ui.js';
import { SYSTEM_PROMPT } from './prompt.js';

// ── Types ──────────────────────────────────────────────────────────

export interface AgentOptions {
  command: string;
  args?: string[];
  maxIterations?: number;
}

// ── Helpers ────────────────────────────────────────────────────────

function buildUserMessage(command: string, args: string[], output: string, iteration: number): string {
  const fullCmd = [command, ...args].join(' ');
  if (iteration === 0) {
    return `Please debug this failing command: \`${fullCmd}\`\n\nError / output:\n\`\`\`\n${output}\n\`\`\``;
  }
  return `The command \`${fullCmd}\` is still failing after the previous fix:\n\`\`\`\n${output}\n\`\`\``;
}

function printSeparator(label: string): void {
  console.log(`\n${'─'.repeat(50)}`);
  console.log(`  ${label}`);
  console.log(`${'─'.repeat(50)}\n`);
}

let toolDefsCache: ToolDefinition[] | null = null;
function getToolDefs(): ToolDefinition[] {
  if (!toolDefsCache) {
    toolDefsCache = getToolDefinitions().map((def) => ({
      type: 'function' as const,
      function: { ...def.function },
    }));
  }
  return toolDefsCache;
}

// ── Agent loop ─────────────────────────────────────────────────────

export async function runAgent(options: AgentOptions): Promise<void> {
  const { command, args = [], maxIterations = 10 } = options;
  const llm = getDefaultClient();

  const messages: ChatMessage[] = [{ role: 'system', content: SYSTEM_PROMPT }];

  printSeparator(`Running: ${command} ${args.join(' ')}`);

  // ── Initial run ──────────────────────────────────────────────
  let ptyResult = await runPty({
    command,
    args,
    onData: (data) => process.stdout.write(data),
  });

  if (ptyResult.exitCode === 0) {
    console.log('\n✓ Command succeeded. Nothing to debug.');
    return;
  }

  messages.push({
    role: 'user',
    content: buildUserMessage(command, args, ptyResult.output, 0),
  });

  // ── ReAct loop ───────────────────────────────────────────────
  for (let i = 0; i < maxIterations; i++) {
    printSeparator(`Agent iteration ${i + 1}/${maxIterations}`);

    // Stream LLM response
    const response = await llm.chatStream(messages, {
      tools: getToolDefs(),
      onEvent: (event: ChatStreamEvent) => {
        if (event.type === 'content') process.stdout.write(event.delta);
        if (event.type === 'thinking') process.stdout.write(event.delta);
      },
    });

    // Record assistant message
    const assistantMsg: ChatMessage = {
      role: 'assistant',
      content: response.content || null,
    };
    if (response.toolCalls) {
      assistantMsg.tool_calls = response.toolCalls.map((tc) => ({
        id: tc.id,
        type: 'function' as const,
        function: { name: tc.name, arguments: JSON.stringify(tc.arguments) },
      }));
    }
    messages.push(assistantMsg);

    // ── Execute tool calls ─────────────────────────────────────
    if (response.toolCalls && response.toolCalls.length > 0) {
      for (const tc of response.toolCalls) {
        console.log(`\n  🔧 ${tc.name}`);

        // Edit-gate for destructive file writes
        if (tc.name === 'patch_file') {
          const confirmed = await askUserConfirmation(tc.arguments);
          if (!confirmed) {
            console.log('  ⏭  Skipped (user declined).');
            messages.push({
              role: 'tool',
              tool_call_id: tc.id,
              content: 'User declined this edit.',
            });
            continue;
          }
        }

        const result = await executeTool(tc.name, tc.arguments as Record<string, unknown>);
        const status = result.success ? '✓' : '✗';
        const preview = result.content.length > 300 ? result.content.slice(0, 300) + '...' : result.content;
        console.log(`  ${status} ${preview.split('\n')[0]}`);

        messages.push({
          role: 'tool',
          tool_call_id: tc.id,
          content: result.content,
        });
      }
      continue; // back to LLM with tool results
    }

    // ── No tool calls — re-run command to verify ──────────────
    printSeparator(`Re-running: ${command} ${args.join(' ')}`);

    ptyResult = await runPty({
      command,
      args,
      onData: (data) => process.stdout.write(data),
    });

    if (ptyResult.exitCode === 0) {
      console.log('\n✓ Command succeeded — bug fixed!');
      return;
    }

    messages.push({
      role: 'user',
      content: buildUserMessage(command, args, ptyResult.output, i + 1),
    });
  }

  console.log(`\n⚠ Max iterations (${maxIterations}) reached. The issue may still be present.`);
  console.log('  Review the changes and try running tbug again, or fix the remaining issues manually.');
}
