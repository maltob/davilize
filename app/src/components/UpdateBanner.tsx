import { useEffect, useState } from "react";

import { registerServiceWorker } from "../web/register";

/** Offers a reload when the service worker has a newer version of the app waiting (web version). */
export function UpdateBanner() {
  const [apply, setApply] = useState<(() => void) | null>(null);

  useEffect(() => {
    registerServiceWorker((fn) => setApply(() => fn));
  }, []);

  if (!apply) return null;
  return (
    <div
      role="status"
      className="fixed bottom-3 left-1/2 z-50 flex -translate-x-1/2 items-center gap-3 rounded-md border border-border bg-surface px-4 py-2 text-sm text-text shadow-lg"
    >
      <span>A new version of Davilize is ready.</span>
      <button
        type="button"
        onClick={apply}
        className="rounded-md bg-accent px-3 py-1 text-sm font-semibold text-accent-text hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
      >
        Reload
      </button>
    </div>
  );
}
