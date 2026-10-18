<script lang="ts">
  import { page } from "$app/state";
</script>

<svelte:head><title>Error {page.status} – Patient Administration System</title></svelte:head>
<h1>{page.status}</h1>
<p class="error">{page.error?.message}</p>
<p><a href="/">Back to the start</a></p>
