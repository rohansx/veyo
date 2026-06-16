#!/usr/bin/env node
// veyo session recorder.
//
// Drives a controllable browser "console" scene through a scripted timeline, capturing
// a PNG frame per logical tick (default 4 fps) AND — because the script knows exactly
// when each meaningful event fires — emitting the ground-truth annotations.jsonl for
// free. Output is a ready-to-score veyo-eval session at fixtures/sessions/<name>/.
//
//   node record-session.mjs [name] [--frames N] [--fps F]
//
// Time is *logical*: frames are captured back-to-back and stamped at i*(1000/fps) ms,
// exactly as veyo-core consumes them, so we don't fight real-time scheduling.

import { chromium } from 'playwright';
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const argv = process.argv.slice(2);
const FLAG_NAMES = ['--frames', '--fps'];
const flagVal = (name, def) => {
  const i = argv.indexOf(name);
  return i >= 0 && i + 1 < argv.length ? argv[i + 1] : def;
};
const positionals = argv.filter((a, idx) => !a.startsWith('--') && !FLAG_NAMES.includes(argv[idx - 1]));
const NAME = positionals[0] || 'scripted-web';
const FPS = Number(flagVal('--fps', '4'));
const TOTAL = Number(flagVal('--frames', '160'));
const INTERVAL = 1000 / FPS;
const W = 1280;
const H = 720;

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(repoRoot, 'fixtures', 'sessions', NAME);

// --- the controllable scene -------------------------------------------------
// 8×8 grid over 1280×720 => each cell is 160×90px. Zones are placed so each event
// changes a *localized* set of cells (header & sidebar stay static -> low emission).
// High-contrast (light) theme: events paint solid, near-saturated blocks so the
// codec's absolute SAD magnitude registers — mirroring how real attention-grabbing
// events (modals, error banners, app switches) actually look.
const SCENE = /* html */ `<!doctype html><html><head><meta charset="utf-8"><style>
  * { margin:0; box-sizing:border-box; font-family: ui-monospace, monospace; }
  html,body { width:${W}px; height:${H}px; background:#ffffff; color:#111; overflow:hidden; }
  #header { position:absolute; top:0; left:0; width:${W}px; height:90px; background:#e6e6e6;
            border-bottom:2px solid #bbb; padding:28px 24px; font-size:22px; color:#222; }
  #sidebar { position:absolute; top:90px; left:0; width:160px; height:630px; background:#eee;
             border-right:2px solid #ccc; padding:16px 12px; font-size:13px; line-height:2.4; color:#444; }
  #main { position:absolute; top:200px; left:200px; width:740px; height:340px; background:#ffffff;
          border:2px solid #ddd; padding:0; overflow:hidden; }
  #error { position:absolute; top:96px; left:160px; width:1120px; height:88px; background:#d11212;
           color:#fff; padding:30px 24px; font-size:20px; display:none; }
  #toast { position:absolute; top:16px; right:24px; width:240px; height:58px; background:#0a7d28;
           color:#fff; border-radius:8px; padding:18px; font-size:16px; display:none; }
  #modal { position:absolute; top:180px; left:340px; width:600px; height:320px; background:#15346e;
           color:#fff; border-radius:10px; padding:32px; font-size:20px;
           box-shadow:0 0 0 2000px rgba(0,0,0,.55); display:none; }
  #term { position:absolute; bottom:0; left:0; width:${W}px; height:180px; background:#ffffff;
          border-top:2px solid #ddd; padding:14px 20px; font-size:15px; line-height:1.5;
          color:#111; white-space:pre; overflow:hidden; }
</style></head><body>
  <div id="header">veyo demo console</div>
  <div id="sidebar">Explorer<br>· src<br>· crates<br>· docs<br>· Cargo.toml</div>
  <div id="main"></div>
  <div id="error">⚠ error: process exited with code 101</div>
  <div id="toast">✓ Deploy succeeded</div>
  <div id="modal"></div>
  <div id="term"></div>
<script>
  const $ = (id) => document.getElementById(id);
  // A solid colored band fills the main panel — a big white -> color change.
  window.scene = {
    ready: true,
    loadMain(step) {
      const bg = ['#ffffff', '#cfd8e6', '#2d3b55'][Math.min(step, 2)];
      const txt = step >= 2 ? '#fff' : '#222';
      $('main').style.background = bg;
      $('main').innerHTML = '<div style="padding:24px;color:' + txt +
        ';font-size:18px;line-height:1.8">PR #1182 · veyo<br>' +
        (step >= 1 ? 'diff --stat<br>12 files changed, 480 insertions(+)' : 'loading…') + '</div>';
    },
    clearMain() { $('main').style.background = '#ffffff'; $('main').innerHTML = ''; },
    showModal() { $('modal').style.display = 'block';
      $('modal').innerHTML = '<b>Overwrite file?</b><br><br>readme.md already exists.<br><br>[ Cancel ]&nbsp;&nbsp;&nbsp;[ Overwrite ]'; },
    hideModal() { $('modal').style.display = 'none'; },
    error(on) { $('error').style.display = on ? 'block' : 'none'; },
    toast(on) { $('toast').style.display = on ? 'block' : 'none'; },
    _log: [],
    log(line) { this._log.push(line); $('term').innerHTML = this._log.slice(-7).join('<br>'); },
    buildOk() {
      $('term').style.background = '#08240f'; $('term').style.color = '#46d369';
      $('term').innerHTML = '$ cargo build<br>   Compiling veyo-core v0.1.0<br>    Finished in 12.24s<br><b>✓ BUILD PASSED</b>';
    },
  };
</script></body></html>`;

// --- the scripted timeline (frame index -> action + optional annotation) ----
// Multi-frame events (a progressive load) share one annotation at completion.
const TIMELINE = [
  { f: 8,  act: ['loadMain', 1] },
  { f: 9,  act: ['loadMain', 2] },
  { f: 10, act: ['loadMain', 3], kind: 'page_loaded',    note: 'main content finished loading' },
  { f: 30, act: ['showModal'],   kind: 'modal_appeared', note: 'overwrite dialog opened' },
  { f: 44, act: ['hideModal'],   kind: 'modal_dismissed',note: 'dialog closed' },
  { f: 58, act: ['error', true], kind: 'error_shown',    note: 'red error banner appeared' },
  { f: 66, act: ['error', false] },
  { f: 78, act: ['log', '$ cargo build'] },
  { f: 80, act: ['log', '   Compiling deps…'] },
  { f: 86, act: ['buildOk'],     kind: 'build_finished', note: 'build passed (green)' },
  { f: 104, act: ['toast', true],kind: 'notification',   note: 'deploy-succeeded toast' },
  { f: 110, act: ['toast', false] },
  { f: 124, act: ['clearMain'] },
  { f: 125, act: ['loadMain', 1] },
  { f: 126, act: ['loadMain', 3], kind: 'content_refreshed', note: 'main panel replaced' },
];

const main = async () => {
  rmSync(outDir, { recursive: true, force: true });
  mkdirSync(join(outDir, 'frames'), { recursive: true });

  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: W, height: H } });
  await page.setContent(SCENE, { waitUntil: 'load' });
  await page.waitForFunction(() => window.scene && window.scene.ready);

  const frames = [];
  const annotations = [];
  for (let i = 0; i < TOTAL; i++) {
    const t_ms = Math.round(i * INTERVAL);
    for (const ev of TIMELINE.filter((e) => e.f === i)) {
      await page.evaluate(([m, arg]) => window.scene[m](arg), ev.act);
      if (ev.kind) annotations.push({ t_ms, kind: ev.kind, surface: 'screen:0', note: ev.note });
    }
    await page.screenshot({ path: join(outDir, 'frames', `${i}.png`), animations: 'disabled' });
    frames.push({ frame_idx: i, t_ms });
  }
  await browser.close();

  const jsonl = (rows) => rows.map((r) => JSON.stringify(r)).join('\n') + '\n';
  writeFileSync(join(outDir, 'frames.jsonl'), jsonl(frames));
  writeFileSync(join(outDir, 'annotations.jsonl'), jsonl(annotations));
  console.log(
    `recorded "${NAME}": ${frames.length} frames @ ${FPS}fps, ${annotations.length} annotations -> ${outDir}`
  );
};

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
