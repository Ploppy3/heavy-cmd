import { afterEach, describe, expect, test } from "vitest";
import { DatabaseSync } from "node:sqlite";
import { cpuTimes, measure, percent, SAMPLES, usageLine } from "../src/cpu.ts";

const dbs: DatabaseSync[] = [];

function samples(): DatabaseSync {
  const db = new DatabaseSync(":memory:");
  db.exec(SAMPLES);
  dbs.push(db);
  return db;
}

const at = (ms: number, pct: number) => ({ busy: (ms * pct) / 100, total: ms });

afterEach(() => {
  for (const db of dbs.splice(0)) db.close();
});

describe("cpu usage", () => {
  test("busy over total between two readings, across every core", () => {
    expect(percent({ busy: 100, total: 1000 }, { busy: 400, total: 2000 })).toBe(30);
    expect(percent({ busy: 0, total: 0 }, { busy: 2, total: 3 })).toBe(66.7);
  });

  test("counters that went back, as after a restart, give no reading", () => {
    expect(percent({ busy: 500, total: 1000 }, { busy: 10, total: 20 })).toBeNull();
    expect(percent({ busy: 0, total: 1000 }, { busy: 0, total: 1000 })).toBeNull();
  });

  test("the usage is over the last 10 seconds of samples kept in heavy.db", () => {
    const db = samples();
    expect(measure(db, 0, at(0, 0))).toBeNull();
    expect(measure(db, 9_999, at(9_999, 0))).toBeNull();
    // From 0 to 10s the counters say 40% busy.
    expect(measure(db, 10_000, { busy: 4_000, total: 10_000 })).toBe(40);
  });

  test("a sample is kept at most every 2 seconds, whoever takes it", () => {
    const db = samples();
    for (const ms of [0, 500, 1_000, 2_000, 3_000, 4_500]) measure(db, ms, at(ms, 50));
    expect((db.prepare("SELECT at FROM samples ORDER BY at").all() as { at: number }[]).map((row) => row.at)).toEqual([0, 2_000, 4_500]);
  });

  test("the newest sample at least 10 seconds old starts the window; one older than 15 seconds is stale", () => {
    const db = samples();
    measure(db, 0, { busy: 0, total: 0 });
    measure(db, 3_000, { busy: 3_000, total: 3_000 }); // 100% busy for the first 3 seconds
    expect(measure(db, 13_000, { busy: 3_000, total: 13_000 })).toBe(0);
    expect(measure(db, 29_000, at(29_000, 50))).toBeNull();
  });

  test("a sample ahead of the clock, which was set back since, is dropped", () => {
    const db = samples();
    measure(db, 20_000, at(20_000, 50));
    measure(db, 5_000, at(5_000, 50));
    expect((db.prepare("SELECT at FROM samples").all() as { at: number }[]).map((row) => row.at)).toEqual([5_000]);
  });

  test("the machine's counters: busy never more than the total", () => {
    const machine = cpuTimes();
    expect(machine.total).toBeGreaterThan(0);
    expect(machine.busy).toBeLessThanOrEqual(machine.total);
  });

  test("shown as the running commands and the usage against the CPU target", () => {
    expect(usageLine(2, 63.4, 80)).toBe("2 running · cpu 63%/80%");
    expect(usageLine(0, null, 80)).toBe("0 running · cpu ?/80%");
  });
});
