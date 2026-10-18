<script lang="ts">
  import "../app.css";
  import type { Snippet } from "svelte";
  import type { LayoutData } from "./$types";

  let { data, children }: { data: LayoutData; children: Snippet } = $props();
</script>

<a class="skip-link" href="#main">Skip to main content</a>
<header class="site">
  <a class="brand" href="/">Patient Administration System</a>
  <nav aria-label="Main">
    {#each data.nav as item (item.href)}
      <a href={item.href}>{item.label}</a>
    {/each}
  </nav>
  {#if data.userName}
    <span>{data.userName}</span>
    <form method="POST" action="/signout"><button type="submit">Sign out</button></form>
  {:else}
    <a href="/signin">Sign in</a>
  {/if}
</header>
<main id="main">{@render children()}</main>
