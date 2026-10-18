<script lang="ts">
  import type { PageData } from "./$types";
  let { data }: { data: PageData } = $props();
</script>

<svelte:head><title>Sign in – Patient Administration System</title></svelte:head>
<h1>Sign in</h1>
{#if data.message}<p class="error" role="alert">{data.message}</p>{/if}
{#if data.enabled}
  <p>Patients can sign in to see their appointments. Staff sign in to manage the hospital.</p>
  <a class="button" href={`/signin/oidc?return=${encodeURIComponent(data.returnTo)}`}>Sign in</a>
{:else}
  <p class="error">Sign-in is not configured on this server (OIDC_ISSUER and OIDC_CLIENT_ID).</p>
{/if}
