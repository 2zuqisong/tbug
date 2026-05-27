#!/usr/bin/env node

async function main(): Promise<void> {
  console.log("TBug v0.1.0 — AI-powered autonomous debugging assistant");
  // TODO: wire up agent loop (agent/loop.ts)
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
