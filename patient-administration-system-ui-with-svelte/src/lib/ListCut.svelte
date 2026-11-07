<!-- Says that a list is not all of it (NFR-23). `at` is the limit that applied, or null for a whole list. -->
<script lang="ts">
  import { t } from "#lib/i18n.svelte.js";

  let { at }: { at: number | null } = $props();
</script>

{#if at !== null}
  <p data-testid="list-cut">{t("list.cut", { count: at })}</p>
{/if}
