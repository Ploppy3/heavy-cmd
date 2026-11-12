/** `42s`, `3m 4s`, `2h 5m`. */
export function duration(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return s % 60 ? `${m}m ${s % 60}s` : `${m}m`;
  return m % 60 ? `${Math.floor(m / 60)}h ${m % 60}m` : `${Math.floor(m / 60)}h`;
}

/** The local time of `at`, `14:02` (`14:02:11` with `seconds`), its date too on another day than `now`'s: `Jan 2 14:02`. */
export function clock(at: number, now: number, seconds = false): string {
  const date = new Date(at);
  const time = date.toTimeString().slice(0, seconds ? 8 : 5);
  if (date.toDateString() === new Date(now).toDateString()) return time;
  return `${date.toLocaleString("en-US", { month: "short" })} ${date.getDate()} ${time}`;
}

/** Whether `pid` names a live process: one this user may not signal lives too. */
export function pidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}
