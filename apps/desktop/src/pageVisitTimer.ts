// How long the person had each visited page shown, for `v_page_visit.duration_ms`.
//
// A visit begins when its page is shown and ends when another page is shown,
// the thread changes or the window goes away. Only time with the window
// visible counts. The visit's id comes from `record_page_visit`, which
// answers after the visit began (and may answer after it ended), so a visit
// that ends first waits for its id and is sent then. A visit whose record
// failed is never identified and never sent.

export interface Visit {
  identify(id: string): void;
}

class OpenVisit implements Visit {
  private id: string | null = null;
  private ms: number | null = null;
  private sent = false;

  constructor(private readonly send: (id: string, ms: number) => void) {}

  identify(id: string): void {
    if (this.id !== null) return;
    this.id = id;
    this.flush();
  }

  close(ms: number): void {
    if (this.ms !== null) return;
    this.ms = Math.round(ms);
    this.flush();
  }

  private flush(): void {
    if (this.sent || this.id === null || this.ms === null) return;
    this.sent = true;
    this.send(this.id, this.ms);
  }
}

export class PageVisitTimer {
  private current: OpenVisit | null = null;
  private shownMs = 0;
  private shownSince: number | null = null;

  constructor(
    private readonly send: (id: string, ms: number) => void,
    private readonly now: () => number = () => performance.now(),
    private visible = true,
  ) {}

  /** A page is shown: the visit before it ends, and this one starts. */
  begin(): Visit {
    this.end();
    this.current = new OpenVisit(this.send);
    this.shownMs = 0;
    this.shownSince = this.visible ? this.now() : null;
    return this.current;
  }

  /** The window was hidden or shown again. */
  setVisible(visible: boolean): void {
    if (visible === this.visible) return;
    this.visible = visible;
    if (!this.current) return;
    if (visible) {
      this.shownSince = this.now();
    } else if (this.shownSince !== null) {
      this.shownMs += this.now() - this.shownSince;
      this.shownSince = null;
    }
  }

  /** The visit shown now ends (another page that isn't recorded, the window closing). */
  end(): void {
    const visit = this.current;
    if (!visit) return;
    this.current = null;
    const open = this.shownSince === null ? 0 : this.now() - this.shownSince;
    this.shownSince = null;
    visit.close(this.shownMs + open);
  }
}
