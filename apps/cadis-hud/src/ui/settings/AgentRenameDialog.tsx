import { useEffect, useState } from "react";
import { normalizeAgentName, useHud } from "../hudState.js";
import { sendAgentRename, sendAgentSpecialistUpdate } from "../cadisActions.js";
import {
  buildCustomSpecialist,
  CUSTOM_SPECIALIST_ID,
  CUSTOM_SPECIALIST_OPTION,
  SPECIALIST_OPTIONS,
  defaultSpecialistForRole,
  specialistOption,
} from "../../lib/agent-specialists.js";

export function AgentRenameDialog() {
  const target = useHud((s) => s.agentRenameTarget);
  const gateway = useHud((s) => s.gateway);
  const agent = useHud((s) => s.agents.find((a) => a.spec.id === s.agentRenameTarget));
  const close = useHud((s) => s.setAgentRenameTarget);
  const [name, setName] = useState("");
  const [specialistId, setSpecialistId] = useState("");
  const [customLabel, setCustomLabel] = useState("");
  const [customPersona, setCustomPersona] = useState("");
  const [warning, setWarning] = useState<string | null>(null);

  useEffect(() => {
    const fallback = defaultSpecialistForRole(agent?.spec.role ?? "");
    const current = agent?.specialist ?? fallback;
    const known = specialistOption(current.id);
    setName(agent?.spec.name ?? "");
    setSpecialistId(known ? current.id : CUSTOM_SPECIALIST_ID);
    setCustomLabel(known ? "" : current.label);
    setCustomPersona(current.persona);
    setWarning(null);
  }, [target, agent?.spec.name, agent?.spec.role, agent?.specialist]);

  if (!target || !agent) return null;
  const disconnected = gateway !== "connected";

  const selectedSpecialist =
    specialistId === CUSTOM_SPECIALIST_ID
      ? buildCustomSpecialist(customLabel, customPersona)
      : specialistOption(specialistId) ?? defaultSpecialistForRole(agent.spec.role);

  const submit = () => {
    if (disconnected) {
      setWarning("CADIS daemon is disconnected. Reconnect first to save agent settings.");
      return;
    }
    const next = normalizeAgentName(name);
    const renameDelivered = sendAgentRename(target, next);
    const specialistDelivered = sendAgentSpecialistUpdate(target, selectedSpecialist);
    const delivered = renameDelivered && specialistDelivered;
    if (delivered) close(null);
    else setWarning("CADIS daemon is not connected. Agent settings will update after daemon confirmation.");
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
          <span className="voice-config__brand">AGENT · SETTINGS</span>
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
            disabled={disconnected}
            onChange={(e) => setName(e.target.value)}
          />
        </section>

        <section className="voice-config__row">
          <label className="voice-config__label" htmlFor="agent-specialist-input">
            Specialist
            <span className="voice-config__value">{selectedSpecialist.label}</span>
          </label>
          <select
            id="agent-specialist-input"
            className="voice-config__select"
            value={specialistId}
            disabled={disconnected}
            onChange={(e) => setSpecialistId(e.target.value)}
          >
            {SPECIALIST_OPTIONS.map((option) => (
              <option key={option.id} value={option.id}>{option.label}</option>
            ))}
            <option value={CUSTOM_SPECIALIST_OPTION.id}>{CUSTOM_SPECIALIST_OPTION.label}</option>
          </select>
        </section>

        {specialistId === CUSTOM_SPECIALIST_ID && (
          <section className="voice-config__row">
            <label className="voice-config__label" htmlFor="agent-specialist-label-input">
              Custom label
              <span className="voice-config__value">{selectedSpecialist.label}</span>
            </label>
            <input
              id="agent-specialist-label-input"
              className="voice-config__input"
              value={customLabel}
              maxLength={48}
              disabled={disconnected}
              onChange={(e) => setCustomLabel(e.target.value)}
            />
          </section>
        )}

        <section className="voice-config__row">
          <label className="voice-config__label" htmlFor="agent-persona-input">
            Persona
            <span className="voice-config__value">{selectedSpecialist.id}</span>
          </label>
          <textarea
            id="agent-persona-input"
            className="voice-config__input voice-config__textarea"
            value={selectedSpecialist.persona}
            rows={5}
            maxLength={1200}
            disabled={disconnected}
            onChange={(e) => {
              if (specialistId !== CUSTOM_SPECIALIST_ID) return;
              setCustomPersona(e.target.value);
            }}
            readOnly={specialistId !== CUSTOM_SPECIALIST_ID}
          />
        </section>

        {warning && <div className="voice-config__hint">{warning}</div>}
        {disconnected && !warning && (
          <div className="voice-config__hint">
            CADIS daemon disconnected. Reconnect to save agent settings.
          </div>
        )}

        <footer className="voice-config__foot">
          <button type="button" className="voice-config__btn" onClick={() => close(null)}>
            CANCEL
          </button>
          <button
            type="submit"
            className="voice-config__btn voice-config__btn--primary"
            disabled={disconnected}
            title={disconnected ? "Daemon disconnected" : undefined}
          >
            SAVE
          </button>
        </footer>
      </form>
    </div>
  );
}
