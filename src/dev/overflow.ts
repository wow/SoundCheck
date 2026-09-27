/**
 * Browser harness only (`pnpm dev:mock`): what does not fit at the current window size. Lists
 * each element whose own children stick out past its edge (spilling, clipped, or scrolling
 * sideways), and each leaf whose text is wider than it, leaving out text cut with an ellipsis
 * by design, canvases, which are drawn to fit, and screen-reader text. The goal is an empty list at 720x560 and every size above.
 */

export interface Overflow {
  /** Tag, id, role or label, and the first classes: enough to find it. */
  element: string;
  /** The start of its text. */
  text: string;
  clientWidth: number;
  scrollWidth: number;
  overflowX: string;
}

export function overflowReport(root: Element = document.body): Overflow[] {
  const view = root.ownerDocument.defaultView;
  if (!view) return [];
  const out: Overflow[] = [];
  for (const el of root.querySelectorAll<HTMLElement>('*')) {
    // A box 1 px wide or less is hidden on purpose (screen-reader text).
    if (el instanceof view.HTMLCanvasElement || el.clientWidth <= 1) continue;
    const style = view.getComputedStyle(el);
    if (style.textOverflow === 'ellipsis') continue;
    // Its own children past its edge (the row, not every ancestor around it), or content wider
    // than a box that clips it (text included) or a leaf's text.
    const edge = el.getBoundingClientRect().left + el.clientLeft + el.clientWidth;
    // Positioned children (a badge's mark, a popover) overhang on purpose.
    const spills = [...el.children].some((c) => {
      const position = view.getComputedStyle(c).position;
      return (
        position !== 'absolute' &&
        position !== 'fixed' &&
        c.getBoundingClientRect().right > edge + 1
      );
    });
    const clips = style.overflowX !== 'visible';
    const wider = (clips || el.children.length === 0) && el.scrollWidth > el.clientWidth + 1;
    if (!spills && !wider) continue;
    out.push({
      element: describe(el),
      text: (el.textContent ?? '').replace(/\s+/g, ' ').trim().slice(0, 60),
      clientWidth: el.clientWidth,
      scrollWidth: el.scrollWidth,
      overflowX: style.overflowX,
    });
  }
  return out;
}

function describe(el: HTMLElement): string {
  const name = el.getAttribute('aria-label') ?? el.getAttribute('role') ?? '';
  const classes = [...el.classList].slice(0, 4).join('.');
  return `${el.tagName.toLowerCase()}${el.id ? `#${el.id}` : ''}${name ? `[${name}]` : ''}${classes ? `.${classes}` : ''}`;
}
