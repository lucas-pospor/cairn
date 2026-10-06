<script lang="ts">
  // Small inline icon set (24x24 stroke icons).
  const paths: Record<string, string> = {
    chevron: "M9 6l6 6-6 6",
    file: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8zM14 3v5h5",
    "file-plus": "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8zM14 3v5h5M12 11v6M9 14h6",
    folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
    "folder-plus": "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2zM12 10v6M9 13h6",
    search: "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4",
    x: "M6 6l12 12M18 6L6 18",
    files: "M8 3h7l4 4v11a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM15 3v4h4M3 8v11a2 2 0 0 0 2 2h9",
    link: "M10 14a4 4 0 0 0 5.66 0l3-3a4 4 0 0 0-5.66-5.66l-1 1M14 10a4 4 0 0 0-5.66 0l-3 3a4 4 0 0 0 5.66 5.66l1-1",
    "panel-left": "M4 4h16v16H4zM9 4v16",
    "panel-right": "M4 4h16v16H4zM15 4v16",
    eye: "M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z",
    code: "M8 7l-5 5 5 5M16 7l5 5-5 5M13.5 5l-3 14",
    tag: "M3 12V4h8l10 10-8 8zM7.5 7.5h.01",
    graph: "M6 6a2 2 0 1 0 0 .01M18 8a2 2 0 1 0 0 .01M12 18a2 2 0 1 0 0 .01M7.6 7.2l2.9 8.9M16.4 9.4l-3.2 7M8 6.4l8 1.2",
    command: "M9 6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3z",
    settings: "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z",
    list: "M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01",
    cloud: "M7 18a4.5 4.5 0 0 1-.5-9 6 6 0 0 1 11.6 1.5A4 4 0 0 1 17.5 18z",
    "cloud-off": "M7 18a4.5 4.5 0 0 1-.5-9 6 6 0 0 1 11.6 1.5A4 4 0 0 1 17.5 18zM4 4l16 16",
    history: "M3 12a9 9 0 1 0 3-6.7M3 4v5h5M12 7v5l3 3",
    pencil: "M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4",
    columns: "M4 4h16v16H4zM12 4v16",
    collapse: "M7 9l5-5 5 5M7 15l5 5 5-5",
    refresh: "M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7",
    vault: "M12 3l9 5-9 5-9-5zM3 13l9 5 9-5M3 18l9 5 9-5",
    arrow: "M5 12h14M13 6l6 6-6 6",
    trash: "M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3",
    template: "M5 4h14v5H5zM5 13h6v7H5zM15 14h4M15 17h4M15 20h4",
  };
  let { name, size = 16 }: { name: string; size?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"><path d={paths[name] ?? ""} /></svg>
