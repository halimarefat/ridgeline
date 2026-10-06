// Pure chart data helpers (no React, unit-tested).

/** Split a [position, value] trace into runs, breaking where positions jump
 *  (skipped intervals), so a line never bridges time that wasn't ridden. */
export function traceRuns(trace: number[][]): number[][][] {
  if (trace.length === 0) return [];
  let step = Infinity;
  for (let i = 1; i < trace.length; i++) step = Math.min(step, trace[i][0] - trace[i - 1][0]);
  if (!isFinite(step) || step <= 0) step = 5;
  const runs: number[][][] = [[trace[0]]];
  for (let i = 1; i < trace.length; i++) {
    if (trace[i][0] - trace[i - 1][0] > step * 1.5) runs.push([]);
    runs[runs.length - 1].push(trace[i]);
  }
  return runs;
}
