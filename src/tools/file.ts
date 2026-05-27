import { readFile } from 'node:fs/promises';
import { applyDelta } from '../utils/diff.js';
import type { ToolDefinition, ToolHandler } from './index.js';

// ── Tool Result ────────────────────────────────────────────────────

export interface ToolResult {
  success: boolean;
  content: string;
}

// ── view_file ──────────────────────────────────────────────────────

interface ViewFileArgs {
  path: string;
  startLine?: number;
  endLine?: number;
}

async function viewFile(args: Record<string, unknown>): Promise<ToolResult> {
  const { path, startLine, endLine } = args as unknown as ViewFileArgs;

  if (!path || typeof path !== 'string') {
    return { success: false, content: 'Error: "path" parameter is required.' };
  }

  let content: string;
  try {
    content = await readFile(path, 'utf-8');
  } catch (err) {
    return {
      success: false,
      content: `Error reading "${path}": ${err instanceof Error ? err.message : String(err)}`,
    };
  }

  const lines = content.split('\n');
  const start = Math.max(1, startLine ?? 1);
  const end = Math.min(lines.length, endLine ?? lines.length);

  if (start > end) {
    return { success: false, content: `Error: startLine (${start}) > endLine (${end}).` };
  }

  const selected = lines.slice(start - 1, end);
  const numWidth = String(end).length;
  const output = selected
    .map((line, i) => `${String(start + i).padStart(numWidth, ' ')}: ${line}`)
    .join('\n');

  return { success: true, content: output };
}

export const viewFileDefinition: ToolDefinition = {
  type: 'function',
  function: {
    name: 'view_file',
    description: 'Read a file from the local filesystem. Returns content with line numbers for the specified range.',
    parameters: {
      type: 'object',
      properties: {
        path: {
          type: 'string',
          description: 'Path to the file to read (relative or absolute).',
        },
        startLine: {
          type: 'integer',
          description: 'Starting line number (1-based, inclusive). Defaults to 1.',
        },
        endLine: {
          type: 'integer',
          description: 'Ending line number (1-based, inclusive). Defaults to the last line.',
        },
      },
      required: ['path'],
    },
  },
};

export const viewFileHandler: ToolHandler = viewFile;

// ── patch_file ─────────────────────────────────────────────────────

interface PatchFileArgs {
  path: string;
  patch: string;
}

async function patchFile(args: Record<string, unknown>): Promise<ToolResult> {
  const { path, patch } = args as unknown as PatchFileArgs;

  if (!path || typeof path !== 'string') {
    return { success: false, content: 'Error: "path" parameter is required.' };
  }
  if (!patch || typeof patch !== 'string') {
    return { success: false, content: 'Error: "patch" parameter is required.' };
  }

  try {
    const result = await applyDelta(path, patch);
    return {
      success: true,
      content: `Successfully applied ${result.applied} SEARCH/REPLACE block(s) to "${path}".`,
    };
  } catch (err) {
    return {
      success: false,
      content: `Error patching "${path}": ${err instanceof Error ? err.message : String(err)}`,
    };
  }
}

export const patchFileDefinition: ToolDefinition = {
  type: 'function',
  function: {
    name: 'patch_file',
    description:
      'Apply one or more SEARCH/REPLACE edits to a file. ' +
      'The patch must use the format:\n' +
      '<<<<<<< SEARCH\n<exact original code>\n=======\n<new code>\n>>>>>>> REPLACE',
    parameters: {
      type: 'object',
      properties: {
        path: {
          type: 'string',
          description: 'Path to the file to modify.',
        },
        patch: {
          type: 'string',
          description:
            'SEARCH/REPLACE block(s). The SEARCH section must match the file content exactly (including whitespace). ' +
            'Add enough surrounding context to make the match unique.',
        },
      },
      required: ['path', 'patch'],
    },
  },
};

export const patchFileHandler: ToolHandler = patchFile;
