import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import "./styles/globals.css";

const root = document.getElementById("root");
if (!root) throw new Error("Missing #root element");

class DiagBoundary extends React.Component<
  { children: React.ReactNode },
  { err: Error | null; info: string }
> {
  state = { err: null as Error | null, info: "" };
  static getDerivedStateFromError(err: Error) {
    return { err, info: "" };
  }
  componentDidCatch(err: Error, info: React.ErrorInfo) {
    this.setState({ err, info: info.componentStack ?? "" });
  }
  render() {
    if (!this.state.err) return this.props.children as React.ReactElement;
    return React.createElement(
      "div",
      {
        style: {
          padding: "20px",
          background: "#222",
          color: "#fdd",
          fontFamily: "monospace",
          fontSize: "12px",
          whiteSpace: "pre-wrap",
          height: "100vh",
          overflow: "auto",
        },
      },
      `RENDER ERROR\n${String(this.state.err.message)}\n\nSTACK:\n${this.state.err.stack ?? ""}\n\nCOMPONENT STACK:${this.state.info}`,
    );
  }
}

window.addEventListener("error", (e) => {
  const r = document.getElementById("root");
  if (r && !r.firstChild)
    r.textContent = `WINDOW ERROR: ${e.message}\n${e.error?.stack ?? ""}`;
});
window.addEventListener("unhandledrejection", (e) => {
  const r = document.getElementById("root");
  const reason = e.reason as { stack?: unknown };
  if (r && !r.firstChild)
    r.textContent = `UNHANDLED REJECTION: ${String(e.reason)}\n${typeof reason.stack === "string" ? reason.stack : ""}`;
});

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <DiagBoundary>
      <App />
    </DiagBoundary>
  </React.StrictMode>,
);
