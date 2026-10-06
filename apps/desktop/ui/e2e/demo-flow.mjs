// End-to-end UI walkthrough against the developer server (no Bluetooth,
// simulated devices): first launch → demo → every screen → coach plan →
// calendar edit → ERG workout ride → stop & save → feedback → history →
// settings. Uses the keyboard for the ride controls (A18) and fails on any
// page error. Screenshots go to $E2E_OUT (default: e2e-out/).
//
// usage: node e2e/demo-flow.mjs [http://localhost:1420]
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const BASE = process.argv[2] ?? "http://localhost:1420";
const OUT = process.env.E2E_OUT ?? "e2e-out";
mkdirSync(OUT, { recursive: true });
const problems = [];
const steps = [];

const browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {});
const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
page.on("pageerror", (e) => problems.push(`page error: ${e.message}`));
page.on("console", (m) => {
  if (m.type() === "error" && !/tiles\.openfreemap|Failed to load resource|ERR_|net::/.test(m.text())) problems.push(`console error: ${m.text()}`);
});

async function step(name, fn) {
  try {
    await fn();
    steps.push(`ok   ${name}`);
  } catch (e) {
    steps.push(`FAIL ${name}: ${String(e.message).split("\n")[0]}`);
    problems.push(`${name}: ${e.message.split("\n")[0]}`);
  }
  await page.screenshot({ path: `${OUT}/${String(steps.length).padStart(2, "0")}-${name.replace(/\W+/g, "-")}.png`, fullPage: false });
}
const nav = async (label) => {
  await page.getByRole("navigation").getByText(label, { exact: true }).first().click();
  await page.waitForTimeout(500);
};
const see = (text) => page.getByText(text, { exact: false }).first().waitFor({ timeout: 10000 });

await page.goto(BASE);
await step("welcome", async () => see("Try a demo"));
await step("start demo", async () => {
  await page.getByRole("button", { name: "Try a demo" }).click();
  await see("Demo mode.");
});
for (const label of ["Devices", "Workouts", "Routes", "Calendar", "Coach", "History", "Settings", "Home"]) {
  await step(`screen ${label}`, async () => {
    await nav(label);
    await page.getByRole("heading", { level: 1 }).first().waitFor();
  });
}
await step("devices ready", async () => {
  await nav("Devices");
  await see("Ridgeline Sim Trainer");
  await page.locator(".dev", { hasText: "Ridgeline Sim Trainer" }).getByText(/\bready\b/).first().waitFor({ timeout: 15000 });
});
await step("coach proposes a plan", async () => {
  await nav("Coach");
  await page.getByRole("button", { name: "Propose a plan" }).click();
  await page.getByRole("dialog").waitFor();
  await see("Offline coach");
});
await step("accept plan", async () => {
  await page.getByRole("button", { name: "Accept" }).first().click();
  await page.getByRole("dialog").waitFor({ state: "detached" });
});
await step("calendar shows plan", async () => {
  await nav("Calendar");
  await page.locator(".cal-session").first().waitFor();
});
await step("edit session → proposal", async () => {
  await page.locator(".cal-session").first().click();
  await page.getByRole("button", { name: "Shorten" }).click();
  await page.getByRole("button", { name: "Preview change" }).click();
  await see("Your change");
  await page.getByRole("button", { name: "Accept" }).first().click();
  await page.getByRole("dialog").waitFor({ state: "detached" });
});
await step("undo plan change", async () => {
  await page.getByRole("button", { name: "Undo last change" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Undo" }).click();
  await see("Change undone.");
});
await step("coach chat (offline)", async () => {
  await nav("Coach");
  await page.getByLabel("Message to the coach").fill("Why is this week structured like this?");
  await page.getByRole("button", { name: "Send" }).click();
  await page.locator(".msg-coach").first().waitFor();
});
await step("launch workout", async () => {
  await nav("Workouts");
  await page.locator(".list-item").first().click();
  await page.getByRole("button", { name: "Ride this workout" }).click();
  await page.getByRole("button", { name: "Start ride" }).waitFor();
  await page.waitForFunction(() => !document.querySelector("button.btn-big[disabled]"), null, { timeout: 15000 });
});
await step("ride runs (keyboard pause/resume)", async () => {
  await page.getByRole("button", { name: "Start ride" }).click();
  await see("trainer controlled");
  await page.waitForTimeout(4000);
  await page.keyboard.press("Space");
  await see("paused");
  await page.keyboard.press("Space");
  await page.waitForTimeout(3000);
});
await step("ride coach: cue, quick prompt by key, suggestion button", async () => {
  await see("Ride cue");
  await page.keyboard.press("1");
  await see("How am I doing?");
  await see("ridden");
  await page.getByRole("button", { name: "Too hard" }).click();
  await see("Ease it 5%");
  await page.getByRole("button", { name: "Easier 5%" }).last().click();
  await see("You chose easier 5%");
  await see("Applied ✓");
  // Typing to the coach doesn't trigger ride shortcuts ("s" would open the stop dialog).
  await page.keyboard.press("c");
  await page.keyboard.type("hello coach, pass the snacks");
  await page.keyboard.press("Enter");
  await see("The offline coach can't chat");
  if (await page.getByRole("dialog").count()) throw new Error("a shortcut fired while typing");
  await page.keyboard.press("Escape");
});
await step("stop and save", async () => {
  await page.keyboard.press("s");
  await page.getByRole("button", { name: "Stop and save" }).click();
  await see("Ride saved");
  await see("The trainer confirmed the stop.");
});
await step("feedback", async () => {
  await page.getByRole("button", { name: "Save feedback" }).click();
  await see("Thanks");
  await page.getByRole("button", { name: "Done" }).click();
});
await step("history shows ride", async () => {
  await nav("History");
  await see("Power, heart rate and cadence");
  await page.getByRole("button", { name: "Export FIT" }).click();
  await see("FIT saved");
});
await step("route detail", async () => {
  await nav("Routes");
  await see("Grade distribution");
});
for (const t of ["FTP", "Trainer & display", "AI coach", "Map & services", "Privacy & data", "About"]) {
  await step(`settings ${t}`, async () => {
    await nav("Settings");
    await page.getByRole("tab", { name: t }).click();
    await page.waitForTimeout(300);
  });
}

await browser.close();
console.log(steps.join("\n"));
if (problems.length) {
  console.log("\nProblems:\n" + problems.map((p) => `::error::${p}`).join("\n"));
  process.exit(1);
}
console.log(`\nAll ${steps.length} steps passed.`);
