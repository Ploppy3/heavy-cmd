import { execFileSync } from "node:child_process";
import path from "node:path";

/** Builds `dist/` before the suites, so `bin/heavy-cmd` runs the source under test. */
export default function build(): void {
  execFileSync("npm", ["run", "--silent", "build"], { cwd: path.join(import.meta.dirname, ".."), stdio: "inherit" });
}
