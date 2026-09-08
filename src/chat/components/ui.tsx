/** Small form/layout primitives shared by every settings pane. */

import { useEffect, useState } from "preact/hooks";
import type { ComponentChildren } from "preact";
import { clamp } from "../../shared/format";

export function Field(props: {
  label?: string;
  hint?: string;
  count?: string;
  children: ComponentChildren;
  class?: string;
}) {
  return (
    <label class={props.class ? `field ${props.class}` : "field"}>
      {props.label ? (
        <span class="field-label">
          <span>{props.label}</span>
          {props.count ? <span class="count">{props.count}</span> : null}
        </span>
      ) : null}
      {props.children}
      {props.hint ? <span class="small muted">{props.hint}</span> : null}
    </label>
  );
}

export function Switch(props: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  hint?: string;
  disabled?: boolean;
}) {
  return (
    <label class="switch" title={props.hint}>
      <input
        type="checkbox"
        checked={props.checked}
        disabled={props.disabled}
        onChange={(event) => props.onChange((event.currentTarget as HTMLInputElement).checked)}
      />
      <span class="switch-track" aria-hidden="true" />
      <span>{props.label}</span>
    </label>
  );
}

export function NumberField(props: {
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  step?: number;
  disabled?: boolean;
  placeholder?: string;
}) {
  const { value, onChange, min, max, step, disabled, placeholder } = props;
  const [focused, setFocused] = useState(false);
  const [draft, setDraft] = useState(String(value));

  useEffect(() => {
    if (!focused) setDraft(String(value));
  }, [value, focused]);

  return (
    <input
      class="input"
      type="number"
      inputMode="decimal"
      value={draft}
      min={min}
      max={max}
      step={step ?? 1}
      disabled={disabled}
      placeholder={placeholder}
      onFocus={() => setFocused(true)}
      onBlur={() => {
        setFocused(false);
        setDraft(String(value));
      }}
      onInput={(event) => {
        const raw = (event.currentTarget as HTMLInputElement).value;
        setDraft(raw);
        const parsed = Number(raw);
        if (raw.trim() !== "" && Number.isFinite(parsed)) {
          onChange(clamp(parsed, min ?? -Infinity, max ?? Infinity));
        }
      }}
    />
  );
}

export function RangeField(props: {
  label: string;
  value: number;
  onChange: (value: number) => void;
  min: number;
  max: number;
  step: number;
  format?: (value: number) => string;
  hint?: string;
  disabled?: boolean;
}) {
  const { label, value, onChange, min, max, step, format, hint, disabled } = props;
  return (
    <div class="field">
      <span class="field-label">
        <span>{label}</span>
        <span class="count">{format ? format(value) : String(value)}</span>
      </span>
      <input
        class="range"
        type="range"
        value={value}
        min={min}
        max={max}
        step={step}
        disabled={disabled}
        aria-label={label}
        onInput={(event) => onChange(Number((event.currentTarget as HTMLInputElement).value))}
      />
      {hint ? <span class="small muted">{hint}</span> : null}
    </div>
  );
}

export function RadioCards<T extends string>(props: {
  value: T;
  onChange: (value: T) => void;
  options: { value: T; title: string; hint?: string }[];
  name: string;
}) {
  return (
    <div class="radio-cards">
      {props.options.map((option) => (
        <label
          key={option.value}
          class={option.value === props.value ? "radio-card checked" : "radio-card"}
        >
          <input
            type="radio"
            name={props.name}
            value={option.value}
            checked={option.value === props.value}
            onChange={() => props.onChange(option.value)}
          />
          <span>
            <span class="radio-card-title">{option.title}</span>
            {option.hint ? <span class="radio-card-hint">{option.hint}</span> : null}
          </span>
        </label>
      ))}
    </div>
  );
}

export function Section(props: {
  title: string;
  desc?: string;
  actions?: ComponentChildren;
  children: ComponentChildren;
}) {
  return (
    <section class="section">
      <div class="section-head">
        <div class="grow">
          <div class="section-title">{props.title}</div>
          {props.desc ? <div class="section-desc">{props.desc}</div> : null}
        </div>
        {props.actions ? <div class="row">{props.actions}</div> : null}
      </div>
      {props.children}
    </section>
  );
}

export function Badge(props: { children: ComponentChildren; kind?: "default" | "accent" | "ok" | "warn" | "danger" }) {
  const kind = props.kind ?? "default";
  return <span class={kind === "default" ? "badge" : `badge badge-${kind}`}>{props.children}</span>;
}

export function EmptyState(props: { children: ComponentChildren }) {
  return <div class="empty">{props.children}</div>;
}
