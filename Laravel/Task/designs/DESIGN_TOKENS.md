# PracticePerfect Design Tokens

Extracted from SVG design files. These tokens are implemented in the application via:
- CSS custom properties, for example `resources/css/app.css`
- Tailwind CSS configuration if the Laravel project uses Tailwind
- Blade, Livewire, Inertia, or frontend components through shared design-token variables
- Dynamic branding through a Laravel config value, database setting, or frontend provider depending on the project architecture

## Typography Scale (from text.svg)

| Token Name | Font Size | Line Height | Weight | Usage |
|------------|-----------|-------------|--------|-------|
| `hero-title` | 30px | 38px | 700 | Page main titles |
| `section-title` | 22px | 28px | 700 | Section headers |
| `block-title` | 18px | 24px | 700 | Card/block headers |
| `card-title` | 16px | 22px | 600 | Card titles |
| `body-lg` | 16px | 26px | 400 | Large body text |
| `body` | 14px | 22px | 400 | Default body text |
| `caption` | 12px | 18px | 400 | Small text, labels |
| `eyebrow` | 11px | 16px | 600 | Uppercase labels |

## Color System (from color_transformation_example*.svg)

### Base Palette (Grayscale)
```
#F3F3F3 → #CFCFCF → #868686 → #5E5E5E → #363636 → #0D0D0D
```

### Accent Transformation Example (Green)
```
#E6FFE6 (lightest) → #99FF99 → #4DFF4D → #00FF00 → #00B300 → #006600 → #001900 (darkest)
```

### Dynamic Color Generation
The branding system generates accent shades programmatically:
- `lightenColor(hex, percent)` - Creates lighter variants
- `darkenColor(hex, percent)` - Creates darker variants
- `hexToRgb(hex)` - Converts for rgba usage

### CSS Custom Properties
```css
--brand-primary: {accentColor}
--brand-primary-soft: lighten(accentColor, 20%)
--brand-primary-deep: darken(accentColor, 20%)
--brand-primary-rgb: r, g, b (for rgba usage)
```

## Spacing Scale

| Token | Value | Usage |
|-------|-------|-------|
| `xxs` | 4px | Tight spacing |
| `xs` | 8px | Small gaps |
| `sm` | 12px | Default small |
| `md` | 16px | Default medium |
| `lg` | 24px | Section padding |
| `xl` | 32px | Large gaps |
| `xxl` | 40px | Extra large |

## Border Radius

| Token | Value | Usage |
|-------|-------|-------|
| `xs` | 6px | Small elements |
| `sm` | 10px | Buttons, inputs |
| `md` | 16px | Cards |
| `lg` | 24px | Large cards |
| `xl` | 32px | Hero sections |
| `pill` | 999px | Pill badges |

## Component Specifications

### Buttons (from buttons.svg)
- **Primary**: Gradient background, glow shadow
- **Secondary/Ghost**: Border outline, transparent
- **Sizes**: sm (padding 18px 8px), md (padding 24px 10px)
- **States**: default, hover (-translate-y-1, scale 1.02), disabled

### Inputs (from inputs.svg)
- **Height**: 40px (10px padding)
- **Border**: 1px solid border-soft
- **Focus**: accent color border, soft glow ring
- **States**: default, focus, error, disabled

### Form Controls (from form_control_element.svg)
- **Checkbox**: 14x14px, rounded-sm
- **Radio**: 22x22px circle
- **Toggle**: 39x24px capsule

## Shadow System

| Token | Value | Usage |
|-------|-------|-------|
| `card-soft` | 0 2px 8px rgba(0,0,0,0.2) | Subtle elevation |
| `card-strong` | 0 4px 16px rgba(0,0,0,0.3) | Hover states |
| `button-primary` | 0 10px 30px rgba(accent, 0.55) | Primary buttons |
| `button-primary-hover` | 0 12px 34px rgba(accent, 0.7) | Button hover |
