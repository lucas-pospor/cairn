// Ad-hoc: open the app on a vault and save a screenshot.  node e2e/shot.mjs <vault> <out.png> [script]
import fs from "node:fs";
import path from "node:path";
import { startDriver, Session } from "./webdriver.mjs";
import { setTimeout as sleep } from "node:timers/promises";

const [vault, out, script] = process.argv.slice(2);
const app = process.env.CAIRN_BIN ?? path.resolve(import.meta.dirname, "../target/debug/cairn");
const drv = await startDriver();
let s;
try {
  s = await Session.create(drv.port, app, [vault]);
  await sleep(1500);
  if (script) console.log(await s.exec(script));
  await sleep(Number(process.env.SHOT_DELAY ?? 500));
  fs.writeFileSync(out, await s.screenshot());
} finally {
  await s?.close();
  drv.proc.kill();
}
