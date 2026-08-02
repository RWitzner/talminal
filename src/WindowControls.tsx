import type { CSSProperties, ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Custom vindueskontroller (minimér/maksimér/luk) til det frameless vindue
 * (decorations: false) — port af Redapting-desktop'ens AppTitleBar-mønster,
 * integreret i Talminals egen topbar i stedet for en separat titelbjælke.
 * Luk er et app-luk: CloseRequested viser én global advarsel og broadcaster
 * derefter `quit_all` til samtlige levende workspace-processer.
 *
 * `getCurrentWindow().close()` bliver staaende efter Task 11: den rammer nu
 * CloseRequested-funnelen, som altid kræver global bekræftelse. Det eneste
 * knappen selv skal vide, er at den er doed MENS en bekraeftelse staar aaben —
 * ellers kan brugeren stable to lukninger oven paa hinanden.
 */
export function WindowControls({
  closeDisabled = false,
}: {
  /** En luk-bekraeftelse er aaben; knappen maa ikke bestille én til. */
  closeDisabled?: boolean;
}) {
  return (
    <div style={rowStyle}>
      <style>{controlsCss}</style>
      <ControlButton
        label="Minimér"
        onClick={() => {
          void getCurrentWindow().minimize();
        }}
      >
        <MinimizeIcon />
      </ControlButton>
      <ControlButton
        label="Maksimér eller gendan"
        onClick={() => {
          void getCurrentWindow().toggleMaximize();
        }}
      >
        <MaximizeIcon />
      </ControlButton>
      <ControlButton
        label="Afslut Talminal"
        disabled={closeDisabled}
        onClick={() => {
          void getCurrentWindow().close();
        }}
      >
        <CloseIcon />
      </ControlButton>
    </div>
  );
}

function ControlButton(props: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={props.label}
      title={props.label}
      data-window-control
      disabled={props.disabled}
      onClick={props.onClick}
    >
      {props.children}
    </button>
  );
}

const rowStyle: CSSProperties = {
  display: "inline-flex",
  flex: "0 0 auto",
  alignItems: "center",
  gap: 2,
  marginLeft: 2,
  paddingLeft: 7,
  borderLeft: "1px solid rgba(209, 232, 251, 0.1)",
};

const controlsCss = `
  [data-window-control] {
    display: grid;
    width: 30px;
    height: 26px;
    place-items: center;
    border: 0;
    border-radius: 7px;
    padding: 0;
    background: transparent;
    color: #9eafc1;
    cursor: pointer;
    transition: color 150ms ease, background 150ms ease;
  }
  [data-window-control]:hover {
    color: #e6f2ff;
    background: rgba(126, 179, 225, 0.11);
  }
  [data-window-control]:focus-visible {
    outline: 2px solid rgba(103, 183, 255, 0.78);
    outline-offset: 2px;
  }
  [data-window-control]:disabled {
    opacity: 0.38;
    cursor: default;
  }
  [data-window-control]:disabled:hover {
    color: #9eafc1;
    background: transparent;
  }
`;

function MinimizeIcon() {
  return (
    <svg
      aria-hidden="true"
      width="14"
      height="14"
      viewBox="0 0 14 14"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
    >
      <path d="M3 8.5h8" />
    </svg>
  );
}

function MaximizeIcon() {
  return (
    <svg
      aria-hidden="true"
      width="14"
      height="14"
      viewBox="0 0 14 14"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinejoin="round"
    >
      <rect x="3.25" y="3.25" width="7.5" height="7.5" rx="1" />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg
      aria-hidden="true"
      width="14"
      height="14"
      viewBox="0 0 14 14"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
    >
      <path d="M4 4l6 6" />
      <path d="M10 4l-6 6" />
    </svg>
  );
}

export default WindowControls;
