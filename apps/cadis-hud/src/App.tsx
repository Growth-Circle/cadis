import { useEffect } from "react";
import { useHud } from "./ui/hudState.js";
import { applyTheme } from "./styles/themes.js";
import { StatusBar } from "./ui/StatusBar.js";
import { OrbitalHUD } from "./ui/orbital/OrbitalHUD.js";
import { ChatPanel } from "./ui/chat/ChatPanel.js";
import { WindowChrome } from "./ui/WindowChrome.js";
import { ConfigDialog } from "./ui/settings/ConfigDialog.js";
import { AgentRenameDialog } from "./ui/settings/AgentRenameDialog.js";
import { ApprovalStack } from "./ui/approvals/ApprovalStack.js";
import { connect, disconnect } from "./ui/cadisActions.js";
import { CodeWorkPanel } from "./ui/codework/CodeWorkPanel.js";

export function App() {
  const theme = useHud((s) => s.theme);
  const backgroundOpacity = useHud((s) => s.backgroundOpacity);

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  useEffect(() => {
    const alpha = backgroundOpacity / 100;
    document.documentElement.style.setProperty("--hud-bg-alpha", String(alpha));
    document.documentElement.style.setProperty("--hud-bg-center-alpha", String(Math.max(0.04, alpha * 0.55)));
    document.documentElement.style.setProperty("--hud-bg-mid-alpha", String(Math.max(0.06, alpha * 0.85)));
    document.documentElement.style.setProperty("--hud-bg-edge-alpha", String(Math.max(0.08, alpha * 0.92)));
    document.documentElement.style.setProperty("--hud-border-alpha", String(Math.max(0.12, alpha * 0.45)));
  }, [backgroundOpacity]);

  useEffect(() => {
    connect();
    return () => disconnect();
  }, []);

  return (
    <div className="rama-shell">
      <WindowChrome />
      <StatusBar />
      <main className="rama-shell__main">
        <OrbitalHUD />
        <ApprovalStack />
        <CodeWorkPanel />
      </main>
      <ChatPanel />
      <ConfigDialog />
      <AgentRenameDialog />
    </div>
  );
}
