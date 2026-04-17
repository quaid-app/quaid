# Design System: Anthropic Claude Docs Aesthetic

This document serves as the visual truth for all generated UI components in the Stitch loop.

## Core Identity
A brutalist-adjacent, exceptionally clean, text-forward design language matching Anthropic's platform documentation.

## Typography
- **Primary Font Family**: `"Inter", -apple-system, system-ui, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif`
- **Headings**: Semi-bold (500-600 weight), tight letter-spacing (`-0.02em` or tighter on larger headings).
- **Body**: Regular weight (400), highly legible line-height (1.6 to 1.8), standard letter-spacing.
- **Monospace**: `ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace`

## Color Palette (Light Mode is Default)
- **Backgrounds**: 
  - Main Canvas (`body`): `#FFFFFF`
  - Sidebars, Navbars, Header: `#FAF9F8` (A signature warm 'paper' off-white)
  - Inline Code Blocks / Subtle Asides: `#F4F3EF`
- **Text**: 
  - Primary text: `#1A1918`
  - Secondary text / subdued: `#3A3836` or `#8A8782`
- **Accents**: 
  - Primary Brand (Buttons, Active States): `#DA7756` (Rust/Coral)
  - Hover states for brand: `#B84A26`
- **Borders & Dividers**:
  - Thin hairlines: `#E8E6E1`

## UI Elements
- **Borders**: All structural boundaries (header bottom, sidebar right, card outlines) must use a 1px solid hairline (`#E8E6E1`). No thick borders.
- **Shadows**: ALMOST NONE. Flat design. Sometimes a microscopic shadow (`0 1px 2px rgba(0,0,0,0.02)`) on cards. Do not use heavy drop shadows.
- **Cards**: Flat white or off-white backgrounds, 1px hairline border, `8px` or `12px` rounded corners.
- **Buttons**: Focus on simple padded rectangles with `8px` border radius. Primary buttons are solid `#DA7756` with white text. Secondary buttons are outline with hairline border and `#1A1918` text, background matching the parent.
- **Icons**: Minimal line icons (SVG). Small footprint (16px x 16px to 20px x 20px). Do not use bold icons.
- **Micro-interactivity**: Keep hover states subtle. Subdue transitions to just un-dimming borders (e.g. from `#E8E6E1` to `#C4CDBC`).

## Structure for Pages
- **Top Navigation**: Fixed, `#FAF9F8` background, bottom hairline border, containing logo on left, search/links centered or right.
- **Hero Sections**: Not artificially padded out; text is aligned left or cleanly centered without massive colored background blobs. Text remains `#1A1918`. Button clusters underneath headline.
- **Sidebar Navigation**: Left aligned, transparent background transitioning to `#F4F3EF` for active selection (no thick left borders). Active text becomes `#DA7756`.

---

### Section 6: Design System Notes for Stitch Generation
[When generating pages via Stitch, ensure the styling precisely matches these specifications: use Tailwind classes like `border-neutral-200` to reflect the hairlines, and `bg-orange-500` mixed contextually to hit the `#DA7756` brand color. Use `font-sans` provided 'Inter' is imported natively natively. Keep backgrounds largely `bg-white` and `bg-stone-50`.]
