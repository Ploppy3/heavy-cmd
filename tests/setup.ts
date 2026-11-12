import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterAll } from "vitest";

// Every suite runs on a temp state and config, never the machine's: a heavy-cmd the suite starts,
// itself run through heavy-cmd, must not join the real queue nor run inside the outer command.
const root = mkdtempSync(path.join(tmpdir(), "heavy-cmd-test-"));
process.env.HEAVY_CMD_HOME = path.join(root, "state");
process.env.XDG_STATE_HOME = path.join(root, "xdg-state");
process.env.XDG_CONFIG_HOME = path.join(root, "xdg-config");
process.env.HEAVY_CMD_AUTO_UPDATE = "off";
for (const name of ["HEAVY_CMD_ID", "HEAVY_CMD_AGENT", "HEAVY_CMD_CPU", "HEAVY_CMD_WAIT", "HEAVY_CMD_RELEASES_API"]) delete process.env[name];

afterAll(() => rmSync(root, { recursive: true, force: true }));
