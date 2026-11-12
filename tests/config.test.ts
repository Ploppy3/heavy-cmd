import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { configFile, ConfigError, DEFAULTS, loadConfig, stateDir } from "../src/config.ts";

let home: string;
let env: NodeJS.ProcessEnv;

/** Writes the config file `env` reads. */
function config(text: string): string {
  const file = configFile(env);
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, text);
  return file;
}

beforeEach(() => {
  home = mkdtempSync(path.join(tmpdir(), "heavy-cmd-config-"));
  env = { HOME: home };
});

afterEach(() => rmSync(home, { recursive: true, force: true }));

describe("where heavy-cmd keeps its files", () => {
  test("the state is $HEAVY_CMD_HOME, else $XDG_STATE_HOME/heavy-cmd, else ~/.local/state/heavy-cmd", () => {
    expect(stateDir(env)).toBe(path.join(home, ".local/state/heavy-cmd"));
    expect(stateDir({ ...env, XDG_STATE_HOME: "/xdg/state" })).toBe("/xdg/state/heavy-cmd");
    expect(stateDir({ ...env, XDG_STATE_HOME: "/xdg/state", HEAVY_CMD_HOME: "/heavy" })).toBe("/heavy");
  });

  test("the config is $XDG_CONFIG_HOME/heavy-cmd/config, else ~/.config/heavy-cmd/config; a relative XDG folder is ignored", () => {
    expect(configFile(env)).toBe(path.join(home, ".config/heavy-cmd/config"));
    expect(configFile({ ...env, XDG_CONFIG_HOME: "/xdg/config" })).toBe("/xdg/config/heavy-cmd/config");
    expect(configFile({ ...env, XDG_CONFIG_HOME: "relative" })).toBe(path.join(home, ".config/heavy-cmd/config"));
  });
});

describe("the settings", () => {
  test("with no config file: cpu 60, wait 900, auto update on", () => {
    expect(loadConfig(env)).toEqual(DEFAULTS);
    expect(DEFAULTS).toEqual({ cpu: 60, wait: 900, autoUpdate: true });
  });

  test("the file's key = value lines set them; comments and blank lines are skipped, the last of a repeated key counts", () => {
    config("# the machine's queue\n\ncpu = 80\nwait=30   # half a minute\nauto_update = off\ncpu = 75\n");
    expect(loadConfig(env)).toEqual({ cpu: 75, wait: 30, autoUpdate: false });
  });

  test("the env overrides the file, and --wait overrides the env; an empty variable is unset", () => {
    config("cpu = 80\nwait = 30\nauto_update = off\n");
    const over = { ...env, HEAVY_CMD_CPU: "40", HEAVY_CMD_WAIT: "60", HEAVY_CMD_AUTO_UPDATE: "on" };
    expect(loadConfig(over)).toEqual({ cpu: 40, wait: 60, autoUpdate: true });
    expect(loadConfig(over, { wait: "5" })).toEqual({ cpu: 40, wait: 5, autoUpdate: true });
    expect(loadConfig({ ...env, HEAVY_CMD_CPU: "" })).toMatchObject({ cpu: 80 });
  });

  test("the bounds are the factory's: cpu 10 to 100, wait 0 to 86400", () => {
    expect(loadConfig({ ...env, HEAVY_CMD_CPU: "10", HEAVY_CMD_WAIT: "0" })).toMatchObject({ cpu: 10, wait: 0 });
    expect(loadConfig({ ...env, HEAVY_CMD_CPU: "100", HEAVY_CMD_WAIT: "86400" })).toMatchObject({ cpu: 100, wait: 86_400 });
    expect(() => loadConfig({ ...env, HEAVY_CMD_CPU: "9" })).toThrow('HEAVY_CMD_CPU must be an integer between 10 and 100, not "9"');
    expect(() => loadConfig({ ...env, HEAVY_CMD_CPU: "101" })).toThrow("HEAVY_CMD_CPU must be an integer between 10 and 100");
    expect(() => loadConfig({ ...env, HEAVY_CMD_CPU: "60.5" })).toThrow("HEAVY_CMD_CPU must be an integer between 10 and 100");
    expect(() => loadConfig({ ...env, HEAVY_CMD_WAIT: "86401" })).toThrow('HEAVY_CMD_WAIT must be an integer number of seconds between 0 and 86400, not "86401"');
    expect(() => loadConfig(env, { wait: "-1" })).toThrow('--wait must be an integer number of seconds between 0 and 86400, not "-1"');
    expect(() => loadConfig({ ...env, HEAVY_CMD_AUTO_UPDATE: "yes" })).toThrow('HEAVY_CMD_AUTO_UPDATE must be on or off, not "yes"');
  });

  test("a bad file is refused, naming its line and why: a value out of bounds, an unknown key, a line that is no setting", () => {
    let file = config("wait = 10\ncpu = 5\n");
    expect(() => loadConfig(env)).toThrow(`${file}:2: cpu must be an integer between 10 and 100, not "5"`);
    file = config("heavy_cpu = 60\n");
    expect(() => loadConfig(env)).toThrow(`${file}:1: unknown key heavy_cpu; the keys are cpu, wait, auto_update`);
    file = config("cpu 60\n");
    expect(() => loadConfig(env)).toThrow(`${file}:1: expected key = value, not "cpu 60"`);
    expect(() => loadConfig(env)).toThrow(ConfigError);
  });

  test("a refusal exits 78, sysexits' EX_CONFIG", () => {
    expect(() => loadConfig({ ...env, HEAVY_CMD_CPU: "x" })).toThrow(expect.objectContaining({ code: 78 }));
  });
});
