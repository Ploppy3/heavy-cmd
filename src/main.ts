import { has, only, parse, UsageError } from "./cli.ts";
import { cmdLogs, cmdRun, cmdStatus } from "./commands.ts";
import { versionLine } from "./install.ts";
import { autoUpdate, githubPorts, update } from "./update.ts";

const USAGE = `heavy-cmd: run a heavy command (an install, a build, a test suite) once the machine has room

  heavy-cmd [--label <text>] [--project <name>] [--wait <s>] [--agent <name>] -- <command>...
      run it once it may start: the machine's oldest waiter first, 10s after the last start and,
      while others run, with the CPU over the last 10s under the target (cpu, 60% unless set);
      one argument runs through sh -c; exits with the command's code, or 75 when it could not
      start within --wait (wait, 900s unless set)
  heavy-cmd status [--json]
      the commands running, the waiters in order, the CPU against its target, the last start
  heavy-cmd logs [--project <name>] [--limit <n>] [--since <ISO time | 1h>] [--json]
      the recorded runs of the last 30 days, the last ended first, 50 unless --limit
  heavy-cmd update [--check]
      move this install to the newest release; --check only says whether one is out
  heavy-cmd --version | --help

Config: \${XDG_CONFIG_HOME:-~/.config}/heavy-cmd/config (cpu, wait, auto_update), overridden by
HEAVY_CMD_CPU, HEAVY_CMD_WAIT, HEAVY_CMD_AUTO_UPDATE. State: $HEAVY_CMD_HOME, else
\${XDG_STATE_HOME:-~/.local/state}/heavy-cmd.`;

const RUN_FLAGS = ["label", "project", "wait", "agent"];

/** Runs heavy-cmd with `argv`, and resolves to the exit code; an error thrown carries its own. */
async function main(argv: string[]): Promise<number> {
  const { positionals, flags, command } = parse(argv);
  const say = (text: string) => (process.stdout.write(`${text}\n`), 0);
  if (has(flags, "version")) return say(versionLine());
  if (has(flags, "help") || positionals[0] === "help") return say(USAGE);
  if (command !== null) {
    if (positionals.length) throw new UsageError(`"${positionals.join(" ")}" before --: the command to run goes after --`);
    if (!command.length) throw new UsageError("nothing to run after --");
    only(flags, "a command", RUN_FLAGS);
    try {
      return await cmdRun(command, flags);
    } finally {
      autoUpdate();
    }
  }
  const [sub, ...rest] = positionals;
  if (sub !== undefined && !["status", "logs", "update"].includes(sub)) {
    throw new UsageError(`unknown command "${sub}": a command to run goes after --, as in heavy-cmd -- ${positionals.join(" ")}`);
  }
  if (rest.length) throw new UsageError(`${sub} takes no "${rest.join(" ")}"`);
  switch (sub) {
    case "status":
      only(flags, "status", ["json"]);
      await cmdStatus(flags);
      return 0;
    case "logs":
      only(flags, "logs", ["project", "limit", "since", "json"]);
      cmdLogs(flags);
      return 0;
    case "update":
      only(flags, "update", ["check"]);
      await update(githubPorts(), has(flags, "check"));
      return 0;
  }
  if (RUN_FLAGS.some((key) => has(flags, key))) throw new UsageError("the command to run goes after --");
  process.stderr.write(`${USAGE}\n`);
  return 64;
}

main(process.argv.slice(2)).then(
  (code) => {
    process.exitCode = code;
  },
  (error: unknown) => {
    process.stderr.write(`heavy-cmd: ${error instanceof Error ? error.message : String(error)}\n`);
    // An error that sets the exit carries it as a number (64, 75, 78); Node's own carry a string (`ENOENT`) and exit 1.
    const code = (error as { code?: unknown } | null)?.code;
    process.exitCode = typeof code === "number" ? code : 1;
  },
);
