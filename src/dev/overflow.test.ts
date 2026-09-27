import { overflowReport } from './overflow';

/** jsdom has no layout: give an element the box a browser would. */
function box(el: Element, left: number, width: number, scrollWidth = width) {
  Object.defineProperty(el, 'clientWidth', { configurable: true, value: width });
  Object.defineProperty(el, 'clientLeft', { configurable: true, value: 0 });
  Object.defineProperty(el, 'scrollWidth', { configurable: true, value: scrollWidth });
  el.getBoundingClientRect = () => ({ left, right: left + width, width }) as DOMRect;
}

describe('the overflow report', () => {
  it('lists the row whose items stick out and the leaf whose text does not fit, not their ancestors', () => {
    document.body.innerHTML = `
      <main id="page"><div role="group" aria-label="Fixes" class="flex h-10"><button>Beat 1 here</button><button>Reset</button></div>
      <div role="cell" class="px-2.5">-14.6 → -11.0</div>
      <div role="cell" class="loudness" style="overflow-x: hidden"><span>-14.6</span> → -11.0</div>
      <span class="truncate" style="text-overflow: ellipsis">A very long title</span>
      <canvas></canvas><span class="sr-only">Filter by name</span></main>`;
    const q = (s: string) => document.querySelector(s) as Element;
    box(q('#page'), 0, 720, 809);
    box(q('[role=group]'), 0, 600, 789);
    box(q('[role=group] button'), 0, 500);
    box(q('[role=group] button:last-child'), 500, 289);
    box(q('[role=cell]'), 0, 118, 126);
    box(q('.loudness'), 0, 118, 126);
    box(q('.loudness span'), 0, 40);
    box(q('.truncate'), 0, 100, 300);
    box(q('canvas'), 0, 700, 900);
    box(q('.sr-only'), 0, 1, 83);
    const report = overflowReport(document.body);
    expect(report.map((o) => o.element)).toEqual([
      'div[Fixes].flex.h-10',
      'div[cell].px-2.5',
      'div[cell].loudness',
    ]);
    expect(report[0]).toMatchObject({ clientWidth: 600, scrollWidth: 789 });
    expect(report[1]?.text).toBe('-14.6 → -11.0');
  });

  it('is empty when everything fits', () => {
    document.body.innerHTML = '<div><button>Play</button></div>';
    box(document.querySelector('div')!, 0, 200);
    box(document.querySelector('button')!, 0, 60);
    expect(overflowReport(document.body)).toEqual([]);
  });
});
