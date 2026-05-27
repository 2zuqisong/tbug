#!/usr/bin/env node

import { runAgent } from './agent/loop.js';

async function main(): Promise<void> {
  const args = process.argv.slice(2);

  if (args.length === 0) {
    console.log('TBug v0.1.0 — AI-powered autonomous debugging assistant');
    console.log();
    console.log('Usage:  tbug <command> [args...]');
    console.log();
    console.log('Examples:');
    console.log('  tbug npm run test');
    console.log('  tbug cargo check');
    console.log('  tbug make');
    console.log();
    console.log('Environment:');
    console.log('  DEEPSEEK_API_KEY   Required. Your DeepSeek API key.');
    console.log('  DEEPSEEK_API_BASE  Optional. Defaults to https://api.deepseek.com/v1');
    process.exit(1);
  }

  const command = args[0];
  const commandArgs = args.slice(1);

  await runAgent({ command, args: commandArgs });
}

main().catch((err) => {
  console.error('Fatal error:', err);
  process.exit(1);
});
