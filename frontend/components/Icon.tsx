import type { SVGProps } from 'react'

// Line icons on a 16x16 grid, drawn with the current text color.
const PATHS = {
  database: (
    <>
      <ellipse cx="8" cy="3.8" rx="5" ry="2" />
      <path d="M3 3.8v8.4c0 1.1 2.2 2 5 2s5-.9 5-2V3.8" />
      <path d="M3 8c0 1.1 2.2 2 5 2s5-.9 5-2" />
    </>
  ),
  schema: (
    <path d="M6 2.5c-1.5 0-2 .7-2 2V6c0 1-.5 2-1.5 2 1 0 1.5 1 1.5 2v1.5c0 1.3.5 2 2 2M10 2.5c1.5 0 2 .7 2 2V6c0 1 .5 2 1.5 2-1 0-1.5 1-1.5 2v1.5c0 1.3-.5 2-2 2" />
  ),
  folder: <path d="M2 4.5c0-.6.4-1 1-1h3l1.5 1.5H13c.6 0 1 .4 1 1V12c0 .6-.4 1-1 1H3c-.6 0-1-.4-1-1z" />,
  table: (
    <>
      <rect x="2.5" y="3" width="11" height="10" rx="1.5" />
      <path d="M2.5 6.5h11M2.5 9.75h11M6.5 6.5v6.5" />
    </>
  ),
  view: (
    <>
      <path d="M1.5 8s2.4-4.5 6.5-4.5S14.5 8 14.5 8 12.1 12.5 8 12.5 1.5 8 1.5 8z" />
      <circle cx="8" cy="8" r="2" />
    </>
  ),
  function: (
    <>
      <path d="M11 2.5c-2.2 0-2.8 1.2-3.1 3.2L6.6 12c-.3 1.3-1 1.8-2.1 1.5" />
      <path d="M5 7h5.5" />
    </>
  ),
  column: (
    <>
      <rect x="4.5" y="2.5" width="7" height="11" rx="1.2" />
      <path d="M4.5 6h7" />
    </>
  ),
  lock: (
    <>
      <rect x="3.5" y="7" width="9" height="6.5" rx="1.5" />
      <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />
    </>
  ),
  key: (
    <>
      <circle cx="5.5" cy="10.5" r="2.7" />
      <path d="M7.4 8.6 13 3M10.8 5.2l1.7 1.7" />
    </>
  ),
  index: <path d="M2.5 4h2M2.5 8h2M2.5 12h2M7 4h6.5M7 8h6.5M7 12h4" />,
  search: (
    <>
      <circle cx="7" cy="7" r="4.5" />
      <path d="M10.5 10.5 14 14" />
    </>
  ),
  history: (
    <>
      <circle cx="8" cy="8" r="5.5" />
      <path d="M8 5v3l2 1.5" />
    </>
  ),
  code: <path d="M6 4 2.5 8 6 12M10 4l3.5 4L10 12" />,
  diagram: (
    <>
      <rect x="2" y="2.5" width="5" height="4" rx="1" />
      <rect x="9" y="9.5" width="5" height="4" rx="1" />
      <path d="M4.5 6.5v5H9" />
    </>
  ),
  plus: <path d="M8 3v10M3 8h10" />,
  info: (
    <>
      <circle cx="8" cy="8" r="6" />
      <path d="M8 7.25v3.75M8 5h.01" />
    </>
  ),
  duplicate: (
    <>
      <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
      <path d="M10.5 3.5v-.5A1.5 1.5 0 0 0 9 1.5H4A1.5 1.5 0 0 0 2.5 3v5A1.5 1.5 0 0 0 4 9.5h.5" />
    </>
  ),
  user: <path d="M8 8a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5zM3 13.5c.6-2.4 2.6-3.8 5-3.8s4.4 1.4 5 3.8" />,
  role: <path d="M8 2.5l4.5 1.7v3.6c0 2.9-2 4.9-4.5 5.7-2.5-.8-4.5-2.8-4.5-5.7V4.2z" />,
  minus: <path d="M3 8h10" />,
  trash: <path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.7 9h5.6l.7-9" />,
  refresh: <path d="M13 8a5 5 0 1 1-1.5-3.5M13 3v2.5h-2.5" />,
  collapse: <path d="M4 7l4-3 4 3M4 12l4-3 4 3" />,
  sidebar: (
    <>
      <rect x="2.5" y="3" width="11" height="10" rx="2" />
      <path d="M6.5 3v10" />
    </>
  ),
  settings: (
    <>
      <path d="M3 4.5h10M3 11.5h10" />
      <circle cx="6" cy="4.5" r="1.6" />
      <circle cx="10" cy="11.5" r="1.6" />
    </>
  ),
  close: <path d="M4 4l8 8M12 4l-8 8" />,
  edit: <path d="M10.5 3 13 5.5 6 12.5H3.5V10z" />,
  pulse: <path d="M2.5 8h3l2-4 2 8 2-4h2" />,
  check: <path d="M3.5 8.5l3 3 6-7" />,
  chevronDown: <path d="M4 6l4 4 4-4" />,
  chevronUp: <path d="M4 10l4-4 4 4" />,
  external: <path d="M6.5 4h5.5v5.5M12 4l-7.5 7.5" />,
  eye: (
    <>
      <path d="M1.5 8s2.4-4.5 6.5-4.5S14.5 8 14.5 8s-2.4 4.5-6.5 4.5S1.5 8 1.5 8Z" />
      <circle cx="8" cy="8" r="2" />
    </>
  ),
  wrap: <path d="M2.5 4h11M2.5 8h9a2 2 0 0 1 0 4H8m1.5-1.5L8 12l1.5 1.5M2.5 12h3" />,
  sortable: <path d="M5 6.5 8 3.5l3 3M5 9.5l3 3 3-3" />,
  play: <path d="M5 3.5v9l7-4.5z" fill="currentColor" />,
  stop: <rect x="4" y="4" width="8" height="8" rx="1.5" fill="currentColor" />,
  download: <path d="M8 2.5v8M4.5 7 8 10.5 11.5 7M3 13.5h10" />,
  upload: <path d="M8 10.5v-8M4.5 6 8 2.5 11.5 6M3 13.5h10" />,
  warning: <path d="M8 2.5 14 13H2zM8 6.5v3M8 11.3v.2" />,
  plan: <path d="M2.5 13.5h11M4.5 11V8M8 11V4.5M11.5 11V6.5" />,
  chevronLeft: <path d="M10 4 6 8l4 4" />,
  chevronRight: <path d="M6 4l4 4-4 4" />,
} as const

export type IconName = keyof typeof PATHS

interface IconProps extends SVGProps<SVGSVGElement> {
  name: IconName
  size?: number
}

export function Icon({ name, size = 16, ...rest }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      style={{ flex: 'none' }}
      {...rest}
    >
      {PATHS[name]}
    </svg>
  )
}
