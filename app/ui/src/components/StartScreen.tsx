// The empty start screen: open a repo, or pick a recent one.
export function StartScreen({ recent, onOpen, onPick, busy, error }: { recent: string[]; onOpen(path: string): void; onPick(): void; busy: boolean; error: string | null }) {
  return (
    <div className="start">
      <div className="start-box panel">
        <h1 className="display-heading">Read a repo summary first</h1>
        <p>Open a repo to see each file as function cards grouped by layer, a dependency map, and every function's call flow, with the code one click away.</p>
        <button type="button" className="iso-btn" onClick={onPick} disabled={busy}>
          Open repo…
        </button>
        {error && <p className="err">{error}</p>}
        {recent.length > 0 && (
          <section aria-labelledby="recent-h">
            <div className="eyebrow" id="recent-h">
              Recent repos
            </div>
            <ul className="recent">
              {recent.map((r) => (
                <li key={r}>
                  <button type="button" className="filelink" disabled={busy} onClick={() => onOpen(r)}>
                    {r}
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
    </div>
  );
}
