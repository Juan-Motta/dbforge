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
  refresh: <path d="M13 8a5 5 0 1 1-1.5-3.5M13 3v2.5h-2.5" />,
  collapse: <path d="M4 7l4-3 4 3M4 12l4-3 4 3" />,
  bell: (
    <>
      <path d="M4 11V7a4 4 0 0 1 8 0v4l1 1.5H3z" />
      <path d="M6.5 14h3" />
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
  play: <path d="M5 3.5v9l7-4.5z" fill="currentColor" />,
  stop: <rect x="4" y="4" width="8" height="8" rx="1.5" fill="currentColor" />,
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
