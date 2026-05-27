import { readFileSync } from 'node:fs';

// ── .env loader (lightweight, no extra dependency) ──────────────────
function loadEnv(): void {
  try {
    const envFile = readFileSync('.env', 'utf-8');
    for (const line of envFile.split('\n')) {
      const trimmed = line.trim();
      if (!trimmed || trimmed.startsWith('#')) continue;
      const eqIdx = trimmed.indexOf('=');
      if (eqIdx === -1) continue;
      const key = trimmed.slice(0, eqIdx).trim();
      if (key) {
        process.env[key] = trimmed.slice(eqIdx + 1).trim();
      }
    }
  } catch {
    // .env file absent or unreadable — that's fine
  }
}
loadEnv();

// ── Types ──────────────────────────────────────────────────────────

export interface ChatMessage {
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string | null;
  tool_calls?: ToolCall[];
  tool_call_id?: string;
  name?: string;
}

export interface ToolCall {
  id: string;
  type: 'function';
  function: {
    name: string;
    arguments: string;
  };
}

export interface ToolDefinition {
  type: 'function';
  function: {
    name: string;
    description: string;
    parameters: Record<string, unknown>;
  };
}

export interface ChatOptions {
  model?: string;
  temperature?: number;
  maxTokens?: number;
  tools?: ToolDefinition[];
  toolChoice?: 'auto' | 'none' | { type: 'function'; function: { name: string } };
  onEvent?: (event: ChatStreamEvent) => void;
}

export type ChatStreamEvent =
  | { type: 'content'; delta: string }
  | { type: 'thinking'; delta: string }
  | { type: 'done' };

export interface ChatResponse {
  content: string;
  toolCalls?: Array<{ id: string; name: string; arguments: unknown }>;
  usage?: { promptTokens: number; completionTokens: number };
}

// ── SSE chunk shapes (internal) ────────────────────────────────────

interface StreamDelta {
  role?: string;
  content?: string;
  reasoning_content?: string;
  tool_calls?: StreamToolCallDelta[];
}

interface StreamToolCallDelta {
  index?: number;
  id?: string;
  type?: 'function';
  function?: { name?: string; arguments?: string };
}

interface StreamChoice {
  index: number;
  delta: StreamDelta;
  finish_reason: string | null;
}

interface StreamChunk {
  choices: StreamChoice[];
  usage?: { prompt_tokens: number; completion_tokens: number; total_tokens: number };
}

// ── LLM Client ─────────────────────────────────────────────────────

export class LLMClient {
  private apiKey: string;
  private baseUrl: string;

  constructor(opts?: { apiKey?: string; baseUrl?: string }) {
    this.apiKey = opts?.apiKey ?? process.env['DEEPSEEK_API_KEY'] ?? '';
    this.baseUrl = opts?.baseUrl ?? process.env['DEEPSEEK_API_BASE'] ?? 'https://api.deepseek.com/v1';
    if (!this.apiKey) {
      throw new Error(
        'DEEPSEEK_API_KEY is required. Set it via environment variable, .env file, or LLMClient constructor.',
      );
    }
  }

  async chatStream(messages: ChatMessage[], options: ChatOptions = {}): Promise<ChatResponse> {
    const { model = 'deepseek-chat', temperature, maxTokens, tools, toolChoice, onEvent } = options;

    const body: Record<string, unknown> = {
      model,
      messages,
      stream: true,
    };
    if (temperature !== undefined) body['temperature'] = temperature;
    if (maxTokens !== undefined) body['max_tokens'] = maxTokens;
    if (tools !== undefined) body['tools'] = tools;
    if (toolChoice !== undefined) body['tool_choice'] = toolChoice;

    const res = await fetch(`${this.baseUrl}/chat/completions`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${this.apiKey}`,
      },
      body: JSON.stringify(body),
    });

    if (!res.ok) {
      const errorBody = await res.text().catch(() => '<unreadable>');
      throw new Error(`DeepSeek API error (${res.status}): ${errorBody}`);
    }

    const reader = res.body!.getReader();
    const decoder = new TextDecoder();

    let content = '';
    let sseBuffer = '';
    const toolAccums = new Map<number, { id: string; name: string; arguments: string }>();
    let usage: ChatResponse['usage'];

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      sseBuffer += decoder.decode(value, { stream: true });
      const lines = sseBuffer.split('\n');
      sseBuffer = lines.pop() ?? '';

      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed || !trimmed.startsWith('data: ')) continue;

        const payload = trimmed.slice(6);
        if (payload === '[DONE]') continue;

        let chunk: StreamChunk;
        try {
          chunk = JSON.parse(payload) as StreamChunk;
        } catch {
          continue;
        }

        const choice = chunk.choices?.[0];
        if (!choice) continue;

        const delta = choice.delta;

        if (delta.reasoning_content) {
          onEvent?.({ type: 'thinking', delta: delta.reasoning_content });
        }

        if (delta.content) {
          content += delta.content;
          onEvent?.({ type: 'content', delta: delta.content });
        }

        if (delta.tool_calls) {
          for (const tc of delta.tool_calls) {
            const idx = tc.index ?? 0;
            let acc = toolAccums.get(idx);
            if (!acc) {
              acc = { id: '', name: '', arguments: '' };
              toolAccums.set(idx, acc);
            }
            if (tc.id) acc.id = tc.id;
            if (tc.function?.name) acc.name += tc.function.name;
            if (tc.function?.arguments) acc.arguments += tc.function.arguments;
          }
        }

        if (chunk.usage) {
          usage = {
            promptTokens: chunk.usage.prompt_tokens,
            completionTokens: chunk.usage.completion_tokens,
          };
        }
      }
    }

    onEvent?.({ type: 'done' });

    const toolCalls =
      toolAccums.size > 0
        ? Array.from(toolAccums.values()).map((acc) => ({
            id: acc.id,
            name: acc.name,
            arguments: safeJsonParse(acc.arguments),
          }))
        : undefined;

    const result: ChatResponse = { content };
    if (toolCalls) result.toolCalls = toolCalls;
    if (usage) result.usage = usage;

    return result;
  }
}

// ── Helpers ────────────────────────────────────────────────────────

function safeJsonParse(raw: string): unknown {
  try {
    return JSON.parse(raw);
  } catch {
    return raw;
  }
}

// ── Default singleton ──────────────────────────────────────────────

let defaultClient: LLMClient | null = null;

export function getDefaultClient(): LLMClient {
  if (!defaultClient) {
    defaultClient = new LLMClient();
  }
  return defaultClient;
}

export async function chatStream(
  messages: ChatMessage[],
  options?: ChatOptions,
): Promise<ChatResponse> {
  return getDefaultClient().chatStream(messages, options);
}
