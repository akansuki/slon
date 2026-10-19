<script lang="ts">
  let { name, size = 16 }: { name: string; size?: number } = $props();

  // 16x16 stroke icons; `fill` ones are solid shapes.
  const icons: Record<string, { d: string; fill?: boolean }> = {
    chevron: { d: 'M6 4l4 4-4 4' },
    server: { d: 'M2.5 3h11v4.2h-11zM2.5 8.8h11V13h-11zM5 5.1h.01M5 10.9h.01' },
    database: {
      d: 'M3 4c0-1.1 2.2-2 5-2s5 .9 5 2-2.2 2-5 2-5-.9-5-2zM3 4v8c0 1.1 2.2 2 5 2s5-.9 5-2V4M3 8c0 1.1 2.2 2 5 2s5-.9 5-2',
    },
    schema: {
      d: 'M2 4.5A1.5 1.5 0 0 1 3.5 3h3L8 4.5h4.5A1.5 1.5 0 0 1 14 6v5.5a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5z',
    },
    table: { d: 'M2.5 3.5h11v9h-11zM2.5 6.5h11M2.5 9.5h11M6.5 3.5v9' },
    view: {
      d: 'M1.5 8S4 3.5 8 3.5 14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8zM8 10a2 2 0 1 0 0-4 2 2 0 0 0 0 4z',
    },
    play: { d: 'M5 3.2v9.6L12.8 8z', fill: true },
    stop: { d: 'M4.5 4.5h7v7h-7z', fill: true },
    plus: { d: 'M8 3.5v9M3.5 8h9' },
    close: { d: 'M4 4l8 8M12 4l-8 8' },
    refresh: { d: 'M13 8a5 5 0 1 1-1.6-3.7M13 2.5v3h-3' },
    plug: { d: 'M6 2v3M10 2v3M4.5 5h7v2.5a3.5 3.5 0 0 1-7 0zM8 11v3' },
    trash: { d: 'M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5M7 7v4M9 7v4' },
    key: { d: 'M10 2.5a3.5 3.5 0 1 1-2.9 5.5L2.5 12.6V14H4v-1.2h1.2V11.6h1.2l1.2-1.2' },
    column: { d: 'M5 2.5h6v11H5zM5 6.2h6M5 9.8h6' },
    index: { d: 'M9 1.8L3.8 9h3.7L7 14.2 12.2 7H8.5z' },
    constraint: { d: 'M8 1.8l5 1.8v4.2c0 3-2.2 5-5 6.4-2.8-1.4-5-3.4-5-6.4V3.6zM5.8 8l1.6 1.6L10.4 6.4' },
    trigger: { d: 'M8 2.2a5.8 5.8 0 1 0 0 11.6A5.8 5.8 0 0 0 8 2.2zM6.7 5.5v5L11 8z' },
    sequence: { d: 'M2.5 4.5h1.5M2.5 8h1.5M2.5 11.5h1.5M6.5 4.5h7M6.5 8h7M6.5 11.5h7' },
    history: { d: 'M2.5 8a5.5 5.5 0 1 0 1.7-4M2.5 2.5v3h3M8 5v3.2l2 1.3' },
    export: { d: 'M8 10V2.5M5 5l3-3 3 3M3 9.5v4h10v-4' },
    copy: { d: 'M5.5 5.5h7v8h-7zM3.5 10.5v-8h7' },
    plan: { d: 'M8 2v3.5M8 5.5H3.5V9M8 5.5h4.5V9M2 9h3v4H2zM11 9h3v4h-3zM6.5 9h3v4h-3z' },
    alert: { d: 'M8 2.2l6.3 11H1.7zM8 6.5v3M8 11.5h.01' },
  };

  const icon = $derived(icons[name] ?? icons.alert);
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill={icon.fill ? 'currentColor' : 'none'}
  stroke={icon.fill ? 'none' : 'currentColor'}
  stroke-width="1.4"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  style="flex:none"
>
  <path d={icon.d} />
</svg>
