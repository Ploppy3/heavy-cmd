export type Flags = Record<string, string[]>;

/** heavy-cmd's own exits, sysexits' codes, so they rarely read as a command's own. */
export const EXIT = { usage: 64, config: 78 } as const;

/** A command line heavy-cmd does not take: exits 64. */
export class UsageError extends Error {
  readonly code = EXIT.usage;
}

/** The flags that take a value; every other known flag is a switch. */
const VALUED = new Set(["label", "project", "wait", "agent", "limit", "since"]);
const SWITCHES = new Set(["json", "check", "help", "version"]);

export type Parsed = {
  positionals: string[];
  flags: Flags;
  /** The words after `--`, the command to run; null with no `--`. */
  command: string[] | null;
};

/** `--key value`, `--key=value`, a switch `--json`, `-h` for `--help` and `-V` for `--version`; repeats accumulate, the last one counts. */
export function parse(argv: string[]): Parsed {
  const positionals: string[] = [];
  const flags: Flags = {};
  for (let i = 0; i < argv.length; i++) {
    const token = argv[i]!;
    if (token === "--") return { positionals, flags, command: argv.slice(i + 1) };
    const short = { "-h": "help", "-V": "version" }[token];
    if (short) {
      (flags[short] ??= []).push("");
      continue;
    }
    if (!token.startsWith("-") || token === "-") {
      positionals.push(token);
      continue;
    }
    const body = token.slice(2);
    const eq = body.indexOf("=");
    const key = eq === -1 ? body : body.slice(0, eq);
    if (!token.startsWith("--") || !(VALUED.has(key) || SWITCHES.has(key))) throw new UsageError(`unknown option ${token}`);
    let value = eq === -1 ? "" : body.slice(eq + 1);
    if (VALUED.has(key) && eq === -1) {
      if (argv[i + 1] === undefined || argv[i + 1] === "--") throw new UsageError(`--${key} needs a value`);
      value = argv[++i]!;
    } else if (SWITCHES.has(key) && eq !== -1) {
      throw new UsageError(`--${key} takes no value`);
    }
    (flags[key] ??= []).push(value);
  }
  return { positionals, flags, command: null };
}

export function one(flags: Flags, key: string): string | undefined {
  const values = flags[key];
  return values?.[values.length - 1];
}

export const has = (flags: Flags, key: string) => key in flags;

/** Refuses any flag `what` does not take. */
export function only(flags: Flags, what: string, allowed: string[]): void {
  const extra = Object.keys(flags).find((key) => !allowed.includes(key));
  if (extra) throw new UsageError(`${what} takes no --${extra}`);
}

/** `--key` as a non-negative integer, `fallback` when it is not given. */
export function integerFlag(flags: Flags, key: string, fallback: number): number {
  const raw = one(flags, key);
  if (raw === undefined || raw === "") return fallback;
  const value = Number(raw);
  if (!/^\d+$/.test(raw.trim()) || !Number.isSafeInteger(value)) throw new UsageError(`--${key} must be a non-negative integer`);
  return value;
}

/** `payload` as JSON with `--json`, else the human text. */
export function json(flags: Flags, payload: unknown, human: () => string): void {
  if (has(flags, "json")) process.stdout.write(`${JSON.stringify(payload, null, 2)}\n`);
  else process.stdout.write(`${human()}\n`);
}
