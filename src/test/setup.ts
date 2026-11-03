// Les vues parlent au Rust par l'IPC de Tauri : on le simule, commande par
// commande, sans lancer l'application.
import { clearMocks, mockConvertFileSrc, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach } from "vitest";

export type Handler = (args: Record<string, unknown>) => unknown;

export const calls: { cmd: string; args: Record<string, unknown> }[] = [];

/** Les commandes absentes répondent `null`, ce qui suffit aux appels secondaires. */
export function backend(handlers: Record<string, Handler>) {
  mockIPC((cmd, args) => {
    const payload = (args ?? {}) as Record<string, unknown>;
    calls.push({ cmd, args: payload });
    const handler = handlers[cmd];
    return handler ? handler(payload) : null;
  });
}

beforeEach(() => {
  calls.length = 0;
  mockWindows("main");
  mockConvertFileSrc("macos");
  // jsdom n'implémente pas les fenêtres modales natives.
  HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
    this.open = true;
  };
  HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
    this.open = false;
  };
});

afterEach(() => {
  clearMocks();
});
