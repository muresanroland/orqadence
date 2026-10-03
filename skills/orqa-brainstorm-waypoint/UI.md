# UI Prototype

Adapted from the prototype skill of [mattpocock/skills](https://github.com/mattpocock/skills) (MIT License, Copyright (c) 2026 Matt Pocock).

Generate **several radically different UI variations** on a single route, switchable from a floating bottom bar. The user flips between variants in the browser, picks one (or steals bits from each), then throws the rest away.

## Where the variants live

Put them on the existing page that would host this, behind a `?variant=` search param: its data fetching, params and auth stay, and only the rendering swaps. Only when no page could plausibly host it, use a throwaway route with `prototype` in its path and the same param.

## Process

### 1. State the question and pick N

Default to **3 variants**. More than 5 stops being radically different and starts being noise, so cap there.

Write down the plan in one line, in the prototype's location or a top-of-file comment:

> "Three variants of the settings page, switchable via `?variant=`, on the existing `/settings` route."

### 2. Generate radically different variants

Draft each variant. Hold each one to:

- The page's purpose and the data it has access to.
- The project's component library / styling system (TailwindCSS, shadcn, MUI, plain CSS, whatever).
- A clear exported component name, e.g. `VariantA`, `VariantB`, `VariantC`.

Variants must be **structurally different**: different layout, different information hierarchy, different primary affordance, not just different colours. Three slightly-tweaked card grids isn't a UI prototype, it's wallpaper. If two drafts come out too similar, redo one with explicit "do not use a card grid" guidance.

### 3. Wire them together

Create a single switcher component on the route:

```tsx
// pseudo-code, adapt to the project's framework
const variants = ['A', 'B', 'C'];
const param = searchParams.get('variant');
const variant = variants.includes(param) ? param : 'A'; // missing or unknown falls back to A
return (
  <>
    {variant === 'A' && <VariantA {...data} />}
    {variant === 'B' && <VariantB {...data} />}
    {variant === 'C' && <VariantC {...data} />}
    <PrototypeSwitcher variants={variants} current={variant} />
  </>
);
```

### 4. Build the floating switcher

A small fixed-position bar at the bottom-centre of the screen with three pieces:

- **Left arrow**: cycles to the previous variant (wraps around).
- **Variant label**: shows the current variant key and, if the variant exports a name, that name too. e.g. `B (Sidebar layout)`.
- **Right arrow**: cycles forward (wraps around).

Behaviour:

- Clicking an arrow updates the URL search param (use the framework's router, e.g. `router.replace` on Next, `navigate` on React Router, etc) so the variant is shareable and reload-stable.
- Keyboard: `←` and `→` arrow keys also cycle. Don't intercept arrow keys when an `<input>`, `<textarea>`, or `[contenteditable]` is focused.
- Visually distinct from the page (e.g. high-contrast pill, subtle shadow) so it's obviously not part of the design being evaluated.

Locate the switcher wherever shared UI lives in the project.

### 5. Capture it

Capture it as [SKILL.md](SKILL.md) step 5.7 says.

## Anti-patterns

- **Variants that differ only in colour or copy.** That's a tweak, not a prototype. Real variants disagree about structure.
- **Sharing too much code between variants.** A shared `<Header>` is fine; a shared `<Layout>` defeats the point. Each variant should be free to throw out the layout.
- **Wiring variants to real mutations.** Read-only prototypes are fine. If a variant needs to mutate, point it at a stub: the question is "what should this look like", not "does the backend work".
