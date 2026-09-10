/**
 * Inline SVG icon set (no icon library). Every icon is a 24x24 stroked glyph so
 * it inherits `currentColor` and stays crisp at 14-18px.
 */

interface IconProps {
  size?: number;
  class?: string;
}

function svgProps(size: number, className?: string) {
  return {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    "stroke-width": 1.8,
    "stroke-linecap": "round",
    "stroke-linejoin": "round",
    class: className ? `icon ${className}` : "icon",
    "aria-hidden": true,
    focusable: "false",
  } as const;
}

export function IconPlus({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}

export function IconSend({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M12 19V5M5.5 11.5 12 5l6.5 6.5" />
    </svg>
  );
}

export function IconStop({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <rect x="7" y="7" width="10" height="10" rx="2" fill="currentColor" stroke="none" />
    </svg>
  );
}

export function IconSettings({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1.03 1.56V21a2 2 0 1 1-4 0v-.09A1.7 1.7 0 0 0 9 19.4a1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-1.56-1.03H3a2 2 0 1 1 0-4h.09A1.7 1.7 0 0 0 4.6 9a1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.7 1.7 0 0 0 9 4.6h.09A1.7 1.7 0 0 0 10.12 3V3a2 2 0 1 1 4 0v.09a1.7 1.7 0 0 0 1.03 1.56 1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.7 1.7 0 0 0 19.4 9v.09a1.7 1.7 0 0 0 1.56 1.03H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.51.88Z" />
    </svg>
  );
}

export function IconSparkles({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M12 3.5 13.6 8.4 18.5 10 13.6 11.6 12 16.5 10.4 11.6 5.5 10l4.9-1.6Z" />
      <path d="M18.5 15.5l.7 2.1 2.1.7-2.1.7-.7 2.1-.7-2.1-2.1-.7 2.1-.7Z" />
    </svg>
  );
}

export function IconChat({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M20 15a3 3 0 0 1-3 3H8l-4 3V6a3 3 0 0 1 3-3h10a3 3 0 0 1 3 3Z" />
    </svg>
  );
}

export function IconTrash({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M4 7h16M10 11v6M14 11v6" />
      <path d="M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12" />
      <path d="M9 7V5a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2" />
    </svg>
  );
}

export function IconClose({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M6 6l12 12M18 6 6 18" />
    </svg>
  );
}

export function IconMenu({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M4 7h16M4 12h16M4 17h16" />
    </svg>
  );
}

export function IconChevronDown({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

export function IconArrowDown({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M12 5v14M5.5 12.5 12 19l6.5-6.5" />
    </svg>
  );
}

export function IconCheck({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="m5 12.5 4.5 4.5L19 7" />
    </svg>
  );
}

export function IconAlert({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M12 4.5 2.8 20h18.4Z" />
      <path d="M12 10v4M12 17.2v.1" />
    </svg>
  );
}

export function IconInfo({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 11v5M12 8v.1" />
    </svg>
  );
}

export function IconRefresh({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M20 11a8 8 0 1 0-2.3 5.7" />
      <path d="M20 5v6h-6" />
    </svg>
  );
}

export function IconExternal({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M14 4h6v6M20 4l-8 8" />
      <path d="M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4" />
    </svg>
  );
}

export function IconFolder({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M3 7a2 2 0 0 1 2-2h3.6l1.8 2H19a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" />
    </svg>
  );
}

export function IconDownload({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M12 4v11M7.5 11 12 15.5 16.5 11" />
      <path d="M4 19h16" />
    </svg>
  );
}

export function IconUpload({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M12 15V4M7.5 8 12 3.5 16.5 8" />
      <path d="M4 19h16" />
    </svg>
  );
}

export function IconUser({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <circle cx="12" cy="8.5" r="3.5" />
      <path d="M5 20a7 7 0 0 1 14 0" />
    </svg>
  );
}

export function IconPuzzle({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M9 4.5a2 2 0 1 1 4 0V6h3a1 1 0 0 1 1 1v3h1.5a2 2 0 1 1 0 4H17v3a1 1 0 0 1-1 1h-3v-1.5a2 2 0 1 0-4 0V18H6a1 1 0 0 1-1-1v-3H6.5a2 2 0 1 0 0-4H5V7a1 1 0 0 1 1-1h3Z" />
    </svg>
  );
}

export function IconDatabase({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <ellipse cx="12" cy="6" rx="7" ry="2.6" />
      <path d="M5 6v6c0 1.44 3.13 2.6 7 2.6s7-1.16 7-2.6V6" />
      <path d="M5 12v6c0 1.44 3.13 2.6 7 2.6s7-1.16 7-2.6v-6" />
    </svg>
  );
}

export function IconCopy({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <rect x="9" y="9" width="11" height="11" rx="2" />
      <path d="M5 15V5a2 2 0 0 1 2-2h10" />
    </svg>
  );
}

export function IconPlay({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M8 5.5 18 12 8 18.5Z" />
    </svg>
  );
}

export function IconPet({ size = 16, class: className }: IconProps) {
  return (
    <svg {...svgProps(size, className)}>
      <path d="M7.5 10.5c1.6 0 2.9-1.6 2.9-3.5S9.1 3.5 7.5 3.5 4.6 5.1 4.6 7s1.3 3.5 2.9 3.5Z" />
      <path d="M16.5 10.5c1.6 0 2.9-1.6 2.9-3.5s-1.3-3.5-2.9-3.5S13.6 5.1 13.6 7s1.3 3.5 2.9 3.5Z" />
      <path d="M12 20c3.2 0 5.4-1.8 5.4-4.2 0-2.3-2.3-4.3-5.4-4.3s-5.4 2-5.4 4.3C6.6 18.2 8.8 20 12 20Z" />
    </svg>
  );
}
