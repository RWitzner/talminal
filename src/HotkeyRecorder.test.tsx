/**
 * @vitest-environment happy-dom
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { HotkeyRecorder } from "./HotkeyRecorder";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.invoke.mockResolvedValue(undefined);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(ui: React.ReactElement) {
  act(() => root.render(ui));
}

async function flush() {
  await act(async () => {});
}

// Efter redesignet 2026-07-29 ER tastekap-visningen knappen, saa dens
// textContent er selve genvejen ("Ctrl", "Shift", …) og ikke en etiket.
// Selektoren er derfor data-attributten, ikke teksten.
function changeButton(): HTMLButtonElement {
  const found = container.querySelector<HTMLButtonElement>(
    "[data-hotkey-change]",
  );
  if (!found) throw new Error("ingen skift-genvej-knap");
  return found;
}

async function startListening() {
  act(() => {
    changeButton().dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
  });
  await flush();
}

function fire(init: KeyboardEventInit & { code: string }) {
  act(() => {
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        bubbles: true,
        cancelable: true,
        ...init,
      }),
    );
  });
}

it("modifiers alene laaser ikke; foerste trigger laaser", async () => {
  const onChange = vi.fn().mockResolvedValue(undefined);
  render(<HotkeyRecorder value="Ctrl+Shift+Space" onChange={onChange} />);
  await startListening();
  fire({ code: "ControlLeft", ctrlKey: true });
  fire({ code: "ShiftLeft", ctrlKey: true, shiftKey: true });
  expect(onChange).not.toHaveBeenCalled();
  fire({ code: "KeyQ", ctrlKey: true, shiftKey: true });
  await flush();
  expect(onChange).toHaveBeenCalledWith("Ctrl+Shift+KeyQ");
});

it("Shift+Escape optages, men bar Escape afbryder", async () => {
  const onChange = vi.fn().mockResolvedValue(undefined);
  render(<HotkeyRecorder value="F9" onChange={onChange} />);
  await startListening();
  fire({ code: "Escape", shiftKey: true });
  await flush();
  expect(onChange).toHaveBeenCalledWith("Shift+Escape");
  onChange.mockClear();
  await startListening();
  fire({ code: "Escape" });
  await flush();
  expect(onChange).not.toHaveBeenCalled();
  expect(container.textContent).not.toContain("Tryk kombinationen");
});

it("hoejre Alt gemmes som Alt og viser noten", async () => {
  const onChange = vi.fn().mockResolvedValue(undefined);
  render(<HotkeyRecorder value="F9" onChange={onChange} />);
  await startListening();
  fire({ code: "AltRight", altKey: true });
  fire({ code: "KeyQ", altKey: true });
  await flush();
  expect(onChange).toHaveBeenCalledWith("Alt+KeyQ");
  expect(container.textContent).toContain("Alt betyder venstre Alt");
});

it("bar skrivetast afvises og lytningen fortsaetter", async () => {
  const onChange = vi.fn().mockResolvedValue(undefined);
  render(<HotkeyRecorder value="F9" onChange={onChange} />);
  await startListening();
  fire({ code: "KeyA" });
  await flush();
  expect(onChange).not.toHaveBeenCalled();
  expect(container.textContent).toContain("mindst én modifier");
  expect(container.textContent).toContain("Tryk kombinationen");
});

it("genoptager foer gemmet", async () => {
  const calls: string[] = [];
  mocks.invoke.mockImplementation(
    async (command: string, args?: { suspended?: boolean }) => {
      calls.push(
        command === "suspend_wake_hotkey"
          ? `suspend:${args?.suspended}`
          : command,
      );
    },
  );
  const onChange = vi.fn(async () => {
    calls.push("save");
  });
  render(<HotkeyRecorder value="F9" onChange={onChange} />);
  await startListening();
  fire({ code: "KeyQ", altKey: true });
  await flush();
  expect(calls).toEqual(["suspend:true", "suspend:false", "save"]);
});

it("tegner hver tast som sin egen kap", async () => {
  render(<HotkeyRecorder value="Ctrl+Shift+Space" onChange={vi.fn()} />);
  await flush();
  const kapper = Array.from(
    container.querySelectorAll("[data-hotkey-value] > span"),
  ).map((el) => el.textContent);
  expect(kapper).toEqual(["Ctrl", "Shift", "Mellemrum"]);
});

// Reglerne er en instruktion til handlingen — permanent er de graa tekst man
// holder op med at se.
it("reglerne staar kun mens der optages", async () => {
  render(<HotkeyRecorder value="F9" onChange={vi.fn()} />);
  await flush();
  expect(container.textContent).not.toContain("mindst én modifier");

  await startListening();
  expect(container.textContent).toContain("mindst én modifier");
});

it("Nulstil tegnes kun naar der er en handler", async () => {
  render(<HotkeyRecorder value="F9" onChange={vi.fn()} />);
  await flush();
  expect(container.querySelector("[data-hotkey-reset]")).toBeNull();

  const nulstil = vi.fn();
  render(<HotkeyRecorder value="F9" onChange={vi.fn()} onReset={nulstil} />);
  await flush();
  const knap = container.querySelector<HTMLButtonElement>(
    "[data-hotkey-reset]",
  )!;
  act(() => knap.click());
  expect(nulstil).toHaveBeenCalledTimes(1);
});

it("genoptager ved unmount", async () => {
  render(<HotkeyRecorder value="F9" onChange={vi.fn()} />);
  await startListening();
  act(() => root.unmount());
  await flush();
  expect(mocks.invoke).toHaveBeenCalledWith("suspend_wake_hotkey", {
    suspended: false,
  });
  root = createRoot(container);
});
