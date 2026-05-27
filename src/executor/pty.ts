import { spawn, IPty } from 'node-pty';

export interface PtyOptions {
  command: string;
  args?: string[];
  cwd?: string;
  env?: Record<string, string>;
  timeout?: number;
  onData?: (data: string) => void;
}

export interface PtyResult {
  output: string;
  exitCode: number;
  signal?: number;
}

export async function run(options: PtyOptions): Promise<PtyResult> {
  const { command, args = [], cwd, env, timeout, onData } = options;

  let pty: IPty;
  try {
    pty = spawn(command, args, {
      cwd,
      env: env ? { ...process.env, ...env } : (process.env as Record<string, string>),
    });
  } catch (err) {
    throw new Error(
      `Failed to spawn "${command}": ${err instanceof Error ? err.message : String(err)}`,
    );
  }

  return new Promise((resolve) => {
    const chunks: string[] = [];
    let timedOut = false;

    const dataDisposable = pty.onData((data: string) => {
      chunks.push(data);
      onData?.(data);
    });

    let timeoutId: ReturnType<typeof setTimeout> | undefined;
    if (timeout) {
      timeoutId = setTimeout(() => {
        timedOut = true;
        pty.kill('SIGKILL');
      }, timeout);
    }

    const exitDisposable = pty.onExit(({ exitCode, signal }) => {
      dataDisposable.dispose();
      exitDisposable.dispose();
      if (timeoutId) clearTimeout(timeoutId);

      resolve({
        output: chunks.join(''),
        exitCode: timedOut ? -1 : exitCode,
        signal: timedOut ? undefined : signal,
      });
    });
  });
}
