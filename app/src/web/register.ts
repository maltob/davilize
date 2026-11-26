// Registers the service worker (production web build only) and reports when a new version of the
// app is waiting. `onUpdate` receives a function that switches to it; the page reloads when the
// switch happens.

import { isWeb } from "./platform";

export function registerServiceWorker(onUpdate: (apply: () => void) => void): void {
  if (!import.meta.env.PROD || !isWeb || !("serviceWorker" in navigator)) return;

  // The first install also takes control of the page; that is not an update, so do not reload.
  const hadController = Boolean(navigator.serviceWorker.controller);
  let reloading = false;
  navigator.serviceWorker.addEventListener("controllerchange", () => {
    if (!hadController || reloading) return;
    reloading = true;
    location.reload();
  });

  const start = () => {
    navigator.serviceWorker
      .register(`${import.meta.env.BASE_URL}sw.js`)
      .then((registration) => {
        const offer = () => {
          const waiting = registration.waiting;
          if (waiting && navigator.serviceWorker.controller) {
            onUpdate(() => waiting.postMessage({ type: "SKIP_WAITING" }));
          }
        };
        offer();
        registration.addEventListener("updatefound", () => {
          const installing = registration.installing;
          installing?.addEventListener("statechange", () => {
            if (installing.state === "installed") offer();
          });
        });
      })
      .catch(() => {
        // No service worker (a private window, an insecure origin): the app works online as before.
      });
  };
  // React mounts after the window has usually finished loading, so do not wait for an event that has passed.
  if (document.readyState === "complete") start();
  else window.addEventListener("load", start);
}
