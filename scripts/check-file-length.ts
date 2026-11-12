#!/usr/bin/env node
// Fails when a source file grows past its line budget. A module that needs more
// than 450 lines is doing more than one job: split it before adding to it.
//
//   npm run check:lines                 every target, default budgets
//   MAX_LINES=300 npm run check:lines   a stricter budget for a change
//
// Only source is budgeted. A suite grows with the cases its module has, and
// splitting it scatters one module's cases across files. The budgets differ
// because a shell script is read top to bottom without imports to navigate.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = process.cwd();
const TARGETS = process.argv.slice(2);
const DIRS = TARGETS.length > 0 ? TARGETS : ["src", "scripts"];

type Budget = { match: (file: string) => boolean; max: number; label: string };

const override = process.env.MAX_LINES ? Number(process.env.MAX_LINES) : null;
const budgets: Budget[] = [
  { match: (file) => /\.tsx?$/.test(file), max: override ?? 450, label: "TypeScript" },
  { match: (file) => file.endsWith(".sh"), max: override ?? 250, label: "shell" },
];

// Files that are data, not logic: a reference document or a generated table is
// allowed to be long, and splitting it would make it harder to read.
const SKIP = new Set(["node_modules", ".git", "references"]);

function walk(dir: string): string[] {
  const files: string[] = [];
  for (const entry of readdirSync(dir)) {
    if (SKIP.has(entry) || entry.startsWith(".")) continue;
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) files.push(...walk(path));
    else if (budgets.some((budget) => budget.match(entry))) files.push(path);
  }
  return files;
}

function budgetFor(file: string): Budget | undefined {
  return budgets.find((budget) => budget.match(file));
}

function countLines(file: string): number {
  const content = readFileSync(file, "utf8");
  return content.split("\n").length - (content.endsWith("\n") ? 1 : 0);
}

const offenders: { file: string; lines: number; max: number }[] = [];
for (const dir of DIRS) {
  for (const file of walk(join(ROOT, dir))) {
    const budget = budgetFor(file)!;
    const lines = countLines(file);
    if (lines > budget.max) offenders.push({ file: relative(ROOT, file), lines, max: budget.max });
  }
}

if (offenders.length === 0) {
  const summary = budgets.map((budget) => `${budget.label} ${budget.max}`).join(", ");
  console.log(`✓ every file is within its budget (${summary})`);
  process.exit(0);
}

offenders.sort((a, b) => b.lines - a.lines);
console.error(`✗ ${offenders.length} file(s) over budget:`);
for (const { file, lines, max } of offenders) {
  console.error(`  ${file} — ${lines} lines (max ${max})`);
}
console.error("\nSplit them: a module that needs more lines is doing more than one job.");
process.exit(1);
