<script lang="ts">
  import type { PageData } from "./$types";
  let { data }: { data: PageData } = $props();
</script>

<svelte:head>
  <title>My messages – Patient Administration System</title>
  <link rel="stylesheet" href="/assets/themes/patient.css" />
</svelte:head>

<h1>My messages</h1>
{#if data.messages.length === 0}
  <p>You have no messages.</p>
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
