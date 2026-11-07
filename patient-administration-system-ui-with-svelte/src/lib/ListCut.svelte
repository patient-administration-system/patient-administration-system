<!--
  Says that a list is not all of it (NFR-23), and links to its next page. `at` is the limit that applied, or
  null for a whole list; `next` the link to the next page, when there is one; `first` the link back to the
  first page, when this is a later one.
-->
<script lang="ts">
  import { t } from "#lib/i18n.svelte.js";

  let { at, next = null, first = null }: { at: number | null; next?: string | null; first?: string | null } = $props();
</script>

{#if at !== null}
  <p data-testid="list-cut">{t("list.cut", { count: at })}</p>
{/if}
{#if next || first}
  <p>
    {#if first}<a href={first} data-testid="list-first">{t("list.first")}</a>{/if}
    {#if next}<a href={next} data-testid="list-next">{t("list.next", { count: at ?? 0 })}</a>{/if}
  </p>
{/if}
