import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { EXIT } from "./cli.ts";

/** A setting that is not one heavy-cmd takes: exits 78. */
export class ConfigError extends Error {
  readonly code = EXIT.config;
}

export type Config = {
  /** The CPU target: the usage, in %, a start waits to be under while others run. */
  cpu: number;
  /** How long, in seconds, a command waits to start before it gives up with 75. */
  wait: number;
  /** Whether an install updates itself once a day, after a command ends. */
  autoUpdate: boolean;
};

type Key = "cpu" | "wait" | "auto_update";

type Setting = { field: keyof Config; env: string; rule: string; read: (raw: string) => number | boolean | undefined };

const integerIn = (min: number, max: number) => (raw: string) => {
  const value = Number(raw);
  return /^\d+$/.test(raw) && value >= min && value <= max ? value : undefined;
};

/** Every key the config file takes, its env override and what it must be. */
const SETTINGS: Record<Key, Setting> = {
  cpu: { field: "cpu", env: "HEAVY_CMD_CPU", rule: "an integer between 10 and 100", read: integerIn(10, 100) },
  wait: { field: "wait", env: "HEAVY_CMD_WAIT", rule: "an integer number of seconds between 0 and 86400", read: integerIn(0, 86_400) },
  auto_update: { field: "autoUpdate", env: "HEAVY_CMD_AUTO_UPDATE", rule: "on or off", read: (raw) => (raw === "on" ? true : raw === "off" ? false : undefined) },
};

export const DEFAULTS: Config = { cpu: 60, wait: 900, autoUpdate: true };

/** An XDG base folder: the env's when it is an absolute path, as the spec asks, else `~/<fallback>`. */
function xdg(env: NodeJS.ProcessEnv, name: string, fallback: string): string {
  const value = env[name];
  return value && path.isAbsolute(value) ? value : path.join(env.HOME || homedir(), fallback);
}

/** The queue and its record: `$HEAVY_CMD_HOME`, else `${XDG_STATE_HOME:-~/.local/state}/heavy-cmd`. */
export const stateDir = (env = process.env) =>
  env.HEAVY_CMD_HOME ? path.resolve(env.HEAVY_CMD_HOME) : path.join(xdg(env, "XDG_STATE_HOME", ".local/state"), "heavy-cmd");

/** `${XDG_CONFIG_HOME:-~/.config}/heavy-cmd/config`. */
export const configFile = (env = process.env) => path.join(xdg(env, "XDG_CONFIG_HOME", ".config"), "heavy-cmd", "config");

/** `value` read as `key`'s, else a ConfigError naming `where` and why. */
function valueOf(key: Key, raw: string, where: string): number | boolean {
  const value = SETTINGS[key].read(raw.trim());
  if (value === undefined) throw new ConfigError(`${where} must be ${SETTINGS[key].rule}, not "${raw}"`);
  return value;
}

/**
 * The config file's `key = value` lines: blank lines and `#` comments are
 * skipped, the last of a repeated key counts. A file that is not there sets
 * nothing; an unknown key, a line that is not `key = value` or a value out of
 * its bounds is refused, naming the line.
 */
export function readConfigFile(file: string): Partial<Config> {
  let text: string;
  try {
    text = readFileSync(file, "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return {};
    throw new ConfigError(`${file}: cannot read it: ${(error as Error).message}`);
  }
  const config: Partial<Record<keyof Config, number | boolean>> = {};
  text.split("\n").forEach((line, index) => {
    const content = line.replace(/#.*/, "").trim();
    if (!content) return;
    const where = `${file}:${index + 1}`;
    const match = /^([A-Za-z_]+)\s*=\s*(.*)$/.exec(content);
    if (!match) throw new ConfigError(`${where}: expected key = value, not "${content}"`);
    const key = match[1]!;
    if (!Object.hasOwn(SETTINGS, key)) throw new ConfigError(`${where}: unknown key ${key}; the keys are ${Object.keys(SETTINGS).join(", ")}`);
    config[SETTINGS[key as Key].field] = valueOf(key as Key, match[2]!, `${where}: ${key}`);
  });
  return config as Partial<Config>;
}

/**
 * The settings in force: the defaults, then the config file, then the env
 * (`HEAVY_CMD_CPU`, `HEAVY_CMD_WAIT`, `HEAVY_CMD_AUTO_UPDATE`; empty is unset),
 * then `flags`, the raw `--wait`.
 */
export function loadConfig(env = process.env, flags: { wait?: string } = {}): Config {
  const config = { ...DEFAULTS, ...readConfigFile(configFile(env)) };
  for (const [key, setting] of Object.entries(SETTINGS) as [Key, Setting][]) {
    const raw = env[setting.env];
    if (raw) (config[setting.field] as number | boolean) = valueOf(key, raw, setting.env);
  }
  if (flags.wait !== undefined) config.wait = valueOf("wait", flags.wait, "--wait") as number;
  return config;
}
