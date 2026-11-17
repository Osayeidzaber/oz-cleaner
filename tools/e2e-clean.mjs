// End-to-end cleaning test through the real UI. Only touches the scratch folder given.
// Run the app with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
const root = process.argv[2];
if (!root || !root.toLowerCase().includes("oz-scratch-fixture")) throw new Error("scratch fixture path required");
const ev = (js) => JSON.parse(execFileSync("node", ["tools/cdp.mjs", "eval", js], { encoding: "utf8" }));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const nav = (t) => ev(`[...document.querySelectorAll('.nav__item')].find(c=>c.textContent.includes('${t}')).click();1`);
const log = (n, ok, extra = "") => { console.log(`${ok ? "PASS" : "FAIL"}  ${n} ${extra}`); if (!ok) process.exitCode = 1; };
const f = (p) => `${root}/${p}`;

nav("Volumes"); await sleep(600);
ev(`(()=>{const i=document.querySelector('.field input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(i,${JSON.stringify(root)});i.dispatchEvent(new Event('input',{bubbles:true}));return 1})()`);
await sleep(200);
ev(`document.querySelector('.field button').click();1`);
await sleep(2500);
log("folder scan finished", /S$/i.test(ev(`document.querySelector('.status').innerText`)));

const add = async (name) => {
  ev(`[...document.querySelectorAll('.trow')].find(r=>r.textContent.includes(${JSON.stringify(name)})).click();1`);
  await sleep(500);
  ev(`[...document.querySelectorAll('.detail .btn')].find(b=>b.textContent.includes('Add to Reclaim')).click();1`);
  await sleep(150);
};
await add("to-hold.bin");
await add("to-recycle.bin");
ev(`[...document.querySelectorAll('.trow')].find(r=>r.textContent.includes('sub')).querySelector('.tw').click();1`);
await sleep(500);
await add("to-delete.bin");

nav("Reclaim"); await sleep(1200);
const pick = (name, label) => ev(`(()=>{const it=[...document.querySelectorAll('.item')].find(i=>i.textContent.includes(${JSON.stringify(name)}));[...it.querySelectorAll('.chip')].find(c=>c.textContent.includes(${JSON.stringify(label)})).click();return 1})()`);
pick("to-hold.bin", "Holding"); await sleep(150);
pick("to-recycle.bin", "Recycle"); await sleep(150);
pick("to-delete.bin", "Delete"); await sleep(150);
log("marker-based dev folder offered", ev(`document.body.innerText.includes('node_modules')`));
log("keep-me.bin not suggested", !ev(`document.querySelector('.page').innerText.includes('keep-me.bin')`));

ev(`[...document.querySelectorAll('.btn')].find(b=>b.textContent.includes('Review')).click();1`);
await sleep(700);
log("confirm lists every method", ev(`(()=>{const t=document.querySelector('.modal').innerText;return t.includes('Delete permanently: 1')&&t.includes('Recycle Bin: 1')&&t.includes('Holding Area: 1')})()`));
log("risky delete requires hold", ev(`!!document.querySelector('.modal .btn--danger')`));
log("nothing changed before confirm", existsSync(f("to-hold.bin")) && existsSync(f("sub/to-delete.bin")));

// A short press must not fire.
ev(`(()=>{const b=document.querySelector('.modal .btn--danger');b.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}));b.dispatchEvent(new PointerEvent('pointerup',{bubbles:true}));return 1})()`);
await sleep(1500);
log("short press does nothing", existsSync(f("sub/to-delete.bin")) && ev(`!!document.querySelector('.modal')`));

ev(`document.querySelector('.modal .btn--danger').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}));1`);
await sleep(3500);
log("hold executes deletion", !existsSync(f("sub/to-delete.bin")));
log("recycled", !existsSync(f("to-recycle.bin")));
log("held moved", !existsSync(f("to-hold.bin")));
log("unselected kept", existsSync(f("keep-me.bin")) && existsSync(f("proj/node_modules/x/a.js")));
console.log("  results:", ev(`document.querySelector('.stats').innerText.split(String.fromCharCode(10)).join(' ')`));

const hold = `${root.slice(0, 2)}/OzHold`;
const ops = readdirSync(hold);
log("holding area created", ops.length === 1, ops[0]);
nav("Ledger"); await sleep(800);
log("ledger shows held", ev(`document.body.innerText.includes('held, not yet freed')`));
ev(`[...document.querySelectorAll('.link')].find(b=>b.textContent==='Restore').click();1`);
await sleep(800);
log("restore returns file", existsSync(f("to-hold.bin")));
log("ledger records the restore", ev(`document.querySelector('.table').innerText.includes('Restored')`));
console.log("  hold folder after restore:", readdirSync(`${hold}/${ops[0]}`));
