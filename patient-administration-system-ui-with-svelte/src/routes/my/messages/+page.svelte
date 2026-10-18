<script lang="ts">
  import type { PageData } from "./$types";
  import { t } from "#lib/i18n.svelte.js";
  let { data }: { data: PageData } = $props();
</script>

<svelte:head>
  <title>{t("myMessages.title")} – {t("app.name")}</title>
  <link rel="stylesheet" href="/assets/themes/patient.css" />
</svelte:head>

<h1>{t("myMessages.title")}</h1>
{#if data.messages.length === 0}
  <p>{t("myMessages.empty")}</p>
{:else}
  <ul class="messages">
    {#each data.messages as m (m.id)}
      <li>
        <h2>{m.rendered_subject}</h2>
        <p><time datetime={m.created_at}>{m.created_at.slice(0, 10)}</time></p>
        <p>{m.rendered_body}</p>
      </li>
    {/each}
  </ul>
{/if}
