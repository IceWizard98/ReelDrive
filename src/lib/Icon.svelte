<script>
  // The app is offline on a stick: no icon font, no sprite, no emoji. Emoji in
  // particular arrive as colour glyphs from the system font and break the
  // neutral palette; these are drawn on one 24px grid at one stroke weight, so
  // every control in a row reads as the same object.
  let { name, size = 20 } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.9"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
  font-family="var(--ui)"
>
  {#if name === "play"}
    <path d="M7 4.5 20 12 7 19.5Z" fill="currentColor" stroke-width="1.6" />
  {:else if name === "pause"}
    <path d="M8.5 5v14M15.5 5v14" stroke-width="3" />
  {:else if name === "back10"}
    <!-- The jump distance belongs in the icon: an unlabelled arrow makes the
         user press it once to find out how far it goes. -->
    <path d="M11.5 5.5V2l-4 3.5 4 3.5V5.5a7 7 0 1 1-7 7" />
    <text
      x="12.5"
      y="15.6"
      text-anchor="middle"
      font-size="9"
      font-weight="700"
      fill="currentColor"
      stroke="none">10</text
    >
  {:else if name === "forward10"}
    <path d="M12.5 5.5V2l4 3.5-4 3.5V5.5a7 7 0 1 0 7 7" />
    <text
      x="11.5"
      y="15.6"
      text-anchor="middle"
      font-size="9"
      font-weight="700"
      fill="currentColor"
      stroke="none">10</text
    >
  {:else if name === "volume"}
    <path d="M4 9.5h3.5L12 5.5v13L7.5 14.5H4Z" fill="currentColor" stroke-width="1.6" />
    <path d="M15.5 9a4.2 4.2 0 0 1 0 6M18.2 6.4a8 8 0 0 1 0 11.2" />
  {:else if name === "mute"}
    <path d="M4 9.5h3.5L12 5.5v13L7.5 14.5H4Z" fill="currentColor" stroke-width="1.6" />
    <path d="M16 9.5 21 15M21 9.5 16 15" />
  {:else if name === "fullscreen"}
    <path d="M3.5 9V3.5H9M15 3.5h5.5V9M20.5 15v5.5H15M9 20.5H3.5V15" />
  {:else if name === "exitFullscreen"}
    <path d="M9 3.5V9H3.5M20.5 9H15V3.5M15 20.5V15h5.5M3.5 15H9v5.5" />
  {:else if name === "close"}
    <path d="M5.5 5.5 18.5 18.5M18.5 5.5 5.5 18.5" />
  {:else if name === "back"}
    <path d="M19 12H5M11 6l-6 6 6 6" />
  {:else if name === "check"}
    <path d="M4.5 12.5 9.5 17.5 19.5 6.5" />
  {/if}
</svg>

<style>
  svg {
    display: block;
    flex: 0 0 auto;
  }
</style>
