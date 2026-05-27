import { readFile, writeFile } from 'node:fs/promises';

// ── Types ──────────────────────────────────────────────────────────

export interface DeltaResult {
  applied: number;
}

// ── SEARCH/REPLACE block ───────────────────────────────────────────

interface Block {
  search: string;
  replace: string;
}

/**
 * Parse one or more SEARCH/REPLACE blocks from a delta string.
 * Format:
 *   <<<<<<< SEARCH
 *   [original code]
 *   =======
 *   [new code]
 *   >>>>>>> REPLACE
 */
function parseBlocks(delta: string): Block[] {
  const blocks: Block[] = [];
  const regex = /<<<<<<< SEARCH\n([\s\S]*?)\n=======\n([\s\S]*?)\n>>>>>>> REPLACE/g;
  let m: RegExpExecArray | null;
  while ((m = regex.exec(delta)) !== null) {
    blocks.push({ search: m[1], replace: m[2] });
  }
  if (blocks.length === 0) {
    throw new Error(
      'No valid SEARCH/REPLACE block found. Expected format:\n' +
        '<<<<<<< SEARCH\n<original code>\n=======\n<new code>\n>>>>>>> REPLACE',
    );
  }
  return blocks;
}

// ── Match & replace ────────────────────────────────────────────────

function locateMatch(content: string, search: string): { start: number; end: number } {
  // 1) exact match
  let start = content.indexOf(search);

  // 2) exact match with trailing newline (search block typically omits it)
  if (start === -1) {
    start = content.indexOf(search + '\n');
  }

  // 3) search may end with \n that the file doesn't have
  if (start === -1 && search.endsWith('\n')) {
    start = content.indexOf(search.slice(0, -1));
  }

  if (start === -1) {
    const preview = search.length > 300 ? search.slice(0, 300) + '\n...(truncated)' : search;
    throw new Error(
      `SEARCH block not found in file. Verify the original code matches exactly (including whitespace).\n\n` +
        `--- SEARCH block ---\n${preview}\n--- end ---`,
    );
  }

  const end = start + search.length;

  // Uniqueness: the *matched substring* in the file must appear only once
  const matched = content.slice(start, end);
  const second = content.indexOf(matched, start + 1);
  if (second !== -1) {
    throw new Error(
      'SEARCH block matches multiple locations in the file. Add more surrounding context lines to make the match unique.',
    );
  }

  return { start, end };
}

function applyBlock(content: string, block: Block): string {
  const { start, end } = locateMatch(content, block.search);
  return content.slice(0, start) + block.replace + content.slice(end);
}

// ── Public API ─────────────────────────────────────────────────────

export async function applyDelta(filePath: string, delta: string): Promise<DeltaResult> {
  const blocks = parseBlocks(delta);
  let content = await readFile(filePath, 'utf-8');

  for (const block of blocks) {
    content = applyBlock(content, block);
  }

  await writeFile(filePath, content, 'utf-8');
  return { applied: blocks.length };
}
