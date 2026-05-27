import type { ToolResult } from './file.js';
import {
  viewFileDefinition,
  viewFileHandler,
  patchFileDefinition,
  patchFileHandler,
} from './file.js';

// ── Types ──────────────────────────────────────────────────────────

export type { ToolResult } from './file.js';

export interface ToolDefinition {
  type: 'function';
  function: {
    name: string;
    description: string;
    parameters: Record<string, unknown>;
  };
}

export type ToolHandler = (args: Record<string, unknown>) => Promise<ToolResult>;

// ── Registry ───────────────────────────────────────────────────────

interface ToolEntry {
  definition: ToolDefinition;
  handler: ToolHandler;
}

const registry = new Map<string, ToolEntry>();

export function registerTool(definition: ToolDefinition, handler: ToolHandler): void {
  if (registry.has(definition.function.name)) {
    throw new Error(`Tool "${definition.function.name}" is already registered.`);
  }
  registry.set(definition.function.name, { definition, handler });
}

export function getToolDefinitions(): ToolDefinition[] {
  return Array.from(registry.values()).map((entry) => entry.definition);
}

export async function executeTool(
  name: string,
  args: Record<string, unknown>,
): Promise<ToolResult> {
  const entry = registry.get(name);
  if (!entry) {
    return { success: false, content: `Unknown tool: "${name}". Available: ${Array.from(registry.keys()).join(', ')}` };
  }
  return entry.handler(args);
}

// ── Built-in tools ─────────────────────────────────────────────────

registerTool(viewFileDefinition, viewFileHandler);
registerTool(patchFileDefinition, patchFileHandler);
