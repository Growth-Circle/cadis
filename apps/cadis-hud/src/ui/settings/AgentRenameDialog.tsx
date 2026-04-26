import { useEffect, useState } from "react";
import { normalizeAgentName, useHud } from "../hudState.js";
import { sendAgentRename } from "../cadisActions.js";

export function AgentRenameDialog() {
  const target = useHud((s) => s.agentRenameTarget);
  const agent = useHud((s) => s.agents.find((a) => a.spec.id === s.agentRenameTarget));
  const close = useHud((s) => s.setAgentRenameTarget);
  const [name, setName] = useState("");
  const [warning, setWarning] = useState<string | null>(null);

  useEffect(() => {
    setName(agent?.spec.name ?? "");
    setWarning(null);
  }, [target, agent?.spec.name]);

  if (!target || !agent) return null;

  const submit = () => {
    const next = normalizeAgentName(name);
    const delivered = sendAgentRename(target, next);
    if (delivered) close(null);
    else setWarning("CADIS daemon is not connected. The display name will update after daemon confirmation.");
  };

  return (
    <div className="modal-backdrop" onClick={() => close(null)}>
      <form
        className="voice-config"
        onClick={(e) => e.stopPropagation()}
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <header className="voice-config__head">
          <span className="voice-config__brand">RENAME · AGENT</span>
          <button
            type="button"
            className="voice-config__close"
            onClick={() => close(null)}
            aria-label="close"
          >
            ×
          </button>
        </header>

        <section className="voice-config__row">
          <label className="voice-config__label" htmlFor="agent-name-input">
            Agent name
            <span className="voice-config__value">{agent.spec.id}</span>
          </label>
          <input
            id="agent-name-input"
            className="voice-config__input"
            value={name}
            maxLength={32}
            autoFocus
            onChange={(e) => setName(e.target.value)}
          />
        </section>

        {warning && <div className="voice-config__hint">{warning}</div>}

        <footer className="voice-config__foot">
          <button type="button" className="voice-config__btn" onClick={() => close(null)}>
            CANCEL
          </button>
          <button type="submit" className="voice-config__btn voice-config__btn--primary">
            SAVE
          </button>
        </footer>
      </form>
    </div>
  );
}
