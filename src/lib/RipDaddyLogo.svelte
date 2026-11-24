<script lang="ts">
  import ripdaddyLogo from "../assets/ripdaddy-logo.png";

  interface Props {
    class?: string;
    alt?: string;
  }

  let { class: className = "h-8 w-8", alt = "RipDaddy" }: Props = $props();
</script>

<img
  src={ripdaddyLogo}
  {alt}
  draggable="false"
  decoding="async"
  class={`shrink-0 select-none object-contain drop-shadow-[0_1px_1.5px_rgba(26,21,19,0.16)] ${className}`}
/>
