import * as readline from 'node:readline';

// ── Helpers ────────────────────────────────────────────────────────

function parsePreviews(patch: string): Array<{ search: string; replace: string }> {
  const blocks: Array<{ search: string; replace: string }> = [];
  const regex = /<<<<<<< SEARCH\n([\s\S]*?)\n=======\n([\s\S]*?)\n>>>>>>> REPLACE/g;
  let m: RegExpExecArray | null;
  while ((m = regex.exec(patch)) !== null) {
    blocks.push({ search: m[1], replace: m[2] });
  }
  return blocks;
}

// ── Edit-Gate ──────────────────────────────────────────────────────

export function askUserConfirmation(patchArgs: unknown): Promise<boolean> {
  const args = patchArgs as { path?: string; patch?: string };
  const path = args.path ?? 'unknown';
  const patch = args.patch ?? '';

  console.log('\n┌──────────────────────────────────────────┐');
  console.log('│  EDIT-GATE  —  Confirm file change       │');
  console.log(`│  File: ${path.padEnd(34)}│`);

  const blocks = parsePreviews(patch);
  if (blocks.length === 0) {
    console.log('│  (raw patch)                              │');
    console.log('│' + patch.slice(0, 40).padEnd(42) + '│');
  } else {
    for (const block of blocks) {
      console.log('├──────────────────────────────────────────┤');
      console.log('│  --- SEARCH ---                          │');
      for (const line of block.search.split('\n').slice(0, 6)) {
        const trimmed = line.slice(0, 38);
        console.log(`│  - ${trimmed.padEnd(38)}│`);
      }
      if (block.search.split('\n').length > 6) {
        console.log('│  - ... (truncated)                       │');
      }
      console.log('│  +++ REPLACE +++                         │');
      for (const line of block.replace.split('\n').slice(0, 6)) {
        const trimmed = line.slice(0, 38);
        console.log(`│  + ${trimmed.padEnd(38)}│`);
      }
      if (block.replace.split('\n').length > 6) {
        console.log('│  + ... (truncated)                       │');
      }
    }
  }
  console.log('└──────────────────────────────────────────┘');
  console.log('Apply this change? (y/n)');

  return new Promise((resolve) => {
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    rl.question('> ', (answer) => {
      rl.close();
      resolve(answer.toLowerCase() === 'y' || answer.toLowerCase() === 'yes');
    });
  });
}
