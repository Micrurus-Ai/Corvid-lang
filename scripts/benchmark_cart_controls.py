"""Compare two revisions of the shipped cart controls on fixed-size DOM fixtures."""
import argparse
import json
import statistics
import subprocess
from pathlib import Path

from playwright.sync_api import sync_playwright


ROOT = Path(__file__).resolve().parents[1]
SOURCE = Path("crates/corvid-codegen-wasm/src")


def controls(revision):
    parts = []
    for filename in ("quantity_field.js", "keyed_list.js"):
        path = SOURCE / filename
        if revision:
            parts.append(subprocess.check_output(["git", "show", f"{revision}:{path.as_posix()}"],
                                                 cwd=ROOT).decode())
        else:
            parts.append((ROOT / path).read_text(encoding="utf-8"))
    return "\n".join(parts)


TRIAL = r"""(size) => {
  const region = document.createElement('div');
  region.dataset.corvidList = '';
  const list = document.createElement('ol');
  const status = document.createElement('span');
  status.setAttribute('role', 'status');
  region.append(list, status);
  list.innerHTML = Array.from({length:size}, (_,i) =>
    '<li data-corvid-key="row' + i + '"><div data-corvid-quantity-field>' +
    '<input data-corvid-quantity="price" value="1" required pattern="[0-9]{1,2}">' +
    '<label hidden><output>450</output></label><span role="alert"></span></div>' +
    '<button data-corvid-move="up" hidden>Earlier</button>' +
    '<button data-corvid-move="down" hidden>Later</button></li>').join('');
  document.body.replaceChildren(region);
  const handlers = {price(q) { return 450n * q; }};
  const names = new Set(['price']);
  bindKeyedList(region);
  for (const field of region.querySelectorAll('[data-corvid-quantity-field]')) {
    bindQuantity(field, handlers, names);
  }
  const index = Math.floor((size-1)/2);
  const row = list.children[index];
  const input = row.querySelector('input');
  const output = row.querySelector('output');
  const event = new Event('input', {bubbles:true});
  const edits = 200;
  function edit(sameTotal) {
    const start = performance.now();
    for (let i = 0; i < edits; i++) {
      input.value = i % 2 ? '1' : (sameTotal ? '01' : '2');
      input.dispatchEvent(event);
    }
    if (output.value !== '450') throw Error('incorrect quantity');
    return performance.now() - start;
  }
  const changed_ms = edit(false);
  const same_total_ms = edit(true);
  input.focus();
  const arrows = ['ArrowDown', 'ArrowUp'].map(key => new KeyboardEvent('keydown',
      {bubbles:true, cancelable:true, altKey:true, key}));
  const moves = 100;
  const start = performance.now();
  for (let i = 0; i < moves; i++) input.dispatchEvent(arrows[i % 2]);
  const moves_ms = performance.now() - start;
  if (list.children[index] !== row || document.activeElement !== input ||
      status.textContent !== 'row'+index+', position '+(index+1)+' of '+size) throw Error('incorrect move');
  // Authored whitespace and edits made by other application code must not skew announcements.
  list.prepend(document.createTextNode('\n'), document.createComment('separator'));
  list.append(row);
  input.focus();
  input.dispatchEvent(arrows[1]);
  if (list.children[size-2] !== row || document.activeElement !== input ||
      status.textContent !== 'row'+index+', position '+(size-1)+' of '+size) throw Error('stale position');
  list.firstChild.remove();
  list.firstChild.remove();
  list.insertBefore(row, list.children[index]);
  input.focus();
  input.scrollIntoView({block:'center'});
  window.announcements = [];
  input.addEventListener('keydown', event => {
    if (!event.isTrusted || !event.altKey || !['ArrowUp','ArrowDown'].includes(event.key)) return;
    const expected = event.key === 'ArrowDown' ? index+1 : index;
    const start = event.timeStamp;
    const observed = new MutationObserver(() => {
      observed.disconnect();
      const elapsed = performance.now() - start;
      if (list.children[expected] !== row || document.activeElement !== input ||
          status.textContent !== 'row'+index+', position '+(expected+1)+' of '+size) throw Error('incorrect keyboard move');
      window.announcements.push(elapsed);
    });
    observed.observe(status, {childList:true, subtree:true});
  });
  return {changed_ms, same_total_ms, moves_ms};
}"""


def main(args):
    code = {"baseline": controls(args.baseline), "candidate": controls(None)}
    samples = []
    with sync_playwright() as playwright:
        browser = getattr(playwright, args.engine).launch(channel=args.channel if args.engine == "chromium" else None)
        try:
            for i in range(-1, args.rounds):
                for size in (2, 1000):
                    for name in (list(code) if i % 2 == 0 else list(reversed(code))):
                        with browser.new_page() as page:
                            page.add_script_tag(content=code[name])
                            sample = page.evaluate(TRIAL, size)
                            # Let the focus and scroll from fixture setup finish before user input.
                            page.evaluate("new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done)))")
                            for key in ["ArrowDown", "ArrowUp"] * 4:
                                page.keyboard.press("Alt+" + key)
                            latencies = page.evaluate("window.announcements")
                            assert len(latencies) == 8, latencies
                            sample["key_to_status_ms"] = statistics.median(latencies)
                            sample["keyboard_samples_ms"] = latencies
                            if i >= 0:
                                samples.append(dict(round=i+1, rows=size, variant=name, **sample))
            version = browser.version
        finally:
            browser.close()
    report = dict(baseline=args.baseline, engine=args.engine, version=version, edits_per_batch=200, moves_per_batch=100,
                  samples=samples)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    for size in (2, 1000):
        for name in code:
            rows = [s for s in samples if s["rows"] == size and s["variant"] == name]
            print(size, name, {k: round(statistics.median(s[k] for s in rows), 2)
                               for k in ("changed_ms", "same_total_ms", "moves_ms", "key_to_status_ms")})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", default="72d5c899")
    parser.add_argument("--channel", default="chrome")
    parser.add_argument("--engine", default="chromium", choices=("chromium", "firefox", "webkit"))
    parser.add_argument("--rounds", type=int, default=12)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive")
    main(args)
