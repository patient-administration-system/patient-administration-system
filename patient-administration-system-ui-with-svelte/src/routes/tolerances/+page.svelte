<script lang="ts">
  import type { ActionData, PageData } from "./$types";
  import { i18n, t } from "#lib/i18n.svelte.js";
  import { RAG_MARK, ragOf, toleranceAmount } from "#lib/views.js";

  let { data, form }: { data: PageData; form: ActionData } = $props();
  const when = (iso: string) => new Intl.DateTimeFormat(i18n.locale, { dateStyle: "medium", timeStyle: "short" }).format(new Date(iso));
  const where = (kind: string, key: string) =>
    kind === "ward" ? t("tol.scope.ward", { name: key }) : kind === "service" ? key : t("tol.scope.all");
  const amount = (v: number, unit: string) => {
    const a = toleranceAmount(v, unit, i18n.locale);
    return a.unitKey ? `${a.text} ${t(a.unitKey)}` : a.text;
  };
  const overall = $derived(ragOf(data.status.overall));
</script>

<svelte:head><title>{t("tol.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("tol.title")}</h1>
<p>{t("tol.intro")}</p>

<nav aria-label={t("tol.window")}>
  <strong>{t("tol.window")}:</strong>
  {#each data.windows as w (w)}
    {#if w === data.days}
      <span aria-current="page">{t(`tol.window.${w}`)}</span>
    {:else}
      <a href="?days={w}">{t(`tol.window.${w}`)}</a>
    {/if}
  {/each}
</nav>

<p>
  {t("tol.overall")}:
  <span class="rag rag-{overall}" data-testid="overall"><span aria-hidden="true">{RAG_MARK[overall]}</span> {t(`tol.rag.${overall}`)}</span>
</p>

{#if data.status.readings.length === 0}
  <p>{t("tol.none")}</p>
{:else}
  <table>
    <caption>{t("tol.caption")}</caption>
    <thead>
      <tr>
        <th scope="col">{t("tol.col.measure")}</th>
        <th scope="col">{t("tol.col.status")}</th>
        <th scope="col">{t("tol.col.now")}</th>
        <th scope="col">{t("tol.col.target")}</th>
        <th scope="col">{t("tol.col.green")}</th>
        <th scope="col">{t("tol.col.red")}</th>
        <th scope="col">{t("tol.col.sample")}</th>
      </tr>
    </thead>
    <tbody>
      {#each data.status.readings as r (r.metric)}
        {@const rag = ragOf(r.rag)}
        <tr data-metric={r.metric}>
          <th scope="row">
            {t(`tol.metric.${r.metric}`)}
            <br /><small>{t(`tol.kind.${r.kind}`)} · {t(`tol.dir.${r.direction}`)}</small>
          </th>
          <td><span class="rag rag-{rag}"><span aria-hidden="true">{RAG_MARK[rag]}</span> {t(`tol.rag.${rag}`)}</span></td>
          <td>{r.value === null ? "–" : amount(r.value, r.unit)}</td>
          <td>{amount(r.target, r.unit)}</td>
          <td>{amount(r.tolerance, r.unit)}</td>
          <td>{amount(r.escalate_beyond, r.unit)}</td>
          <td>{r.sample}</td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

{#snippet scoped(list: PageData["status"]["by_service"], firstCol: string, id: string)}
  <table data-scoped={id}>
    <caption>{t(`tol.by_${id}`)}</caption>
    <thead>
      <tr>
        <th scope="col">{t(firstCol)}</th>
        <th scope="col">{t("tol.col.measure")}</th>
        <th scope="col">{t("tol.col.status")}</th>
        <th scope="col">{t("tol.col.now")}</th>
        <th scope="col">{t("tol.col.target")}</th>
        <th scope="col">{t("tol.col.green")}</th>
        <th scope="col">{t("tol.col.red")}</th>
        <th scope="col">{t("tol.col.sample")}</th>
      </tr>
    </thead>
    <tbody>
      {#each list as r (r.scope_key + r.metric)}
        {@const rag = ragOf(r.rag)}
        <tr data-scope-key={r.scope_key} data-scoped-metric={r.metric}>
          <th scope="row">{r.scope_key}</th>
          <td>{t(`tol.metric.${r.metric}`)}</td>
          <td><span class="rag rag-{rag}"><span aria-hidden="true">{RAG_MARK[rag]}</span> {t(`tol.rag.${rag}`)}</span></td>
          <td>{r.value === null ? "–" : amount(r.value, r.unit)}</td>
          <td>{amount(r.target, r.unit)}</td>
          <td>{amount(r.tolerance, r.unit)}</td>
          <td>{amount(r.escalate_beyond, r.unit)}</td>
          <td>{r.sample}</td>
        </tr>
      {/each}
    </tbody>
  </table>
{/snippet}

{#if data.status.by_service.length > 0 || data.status.by_ward.length > 0}
  <p>{t("tol.by.intro")}</p>
  {#if data.status.by_service.length > 0}
    {@render scoped(data.status.by_service, "tol.col.service", "service")}
  {/if}
  {#if data.status.by_ward.length > 0}
    {@render scoped(data.status.by_ward, "tol.col.ward", "ward")}
  {/if}
{/if}

<h2>{t("tol.alerts.title")}</h2>
<p>{t("tol.alerts.intro")}</p>
{#if form?.acknowledged}
  <p role="status">{t("tol.alerts.done")}</p>
{:else if form?.messageKey}
  <p role="alert">{t(form.messageKey)}</p>
{/if}
{#if data.alerts.length === 0}
  <p>{t("tol.alerts.none")}</p>
{:else}
  <table data-alerts>
    <caption>{t("tol.alerts.title")}</caption>
    <thead>
      <tr>
        <th scope="col">{t("tol.col.measure")}</th>
        <th scope="col">{t("tol.col.where")}</th>
        <th scope="col">{t("tol.col.now")}</th>
        <th scope="col">{t("tol.col.red")}</th>
        <th scope="col">{t("tol.alerts.raised")}</th>
        <th scope="col">{t("tol.alerts.ack")}</th>
      </tr>
    </thead>
    <tbody>
      {#each data.alerts as a (a.id)}
        <tr data-alert={a.id}>
          <th scope="row">{t(`tol.metric.${a.metric}`)}</th>
          <td>{where(a.scope_kind, a.scope_key)}</td>
          <td>{amount(a.value, a.unit ?? "")}</td>
          <td>{amount(a.escalate_beyond, a.unit ?? "")}</td>
          <td>{when(a.raised_at)}</td>
          <td>
            {#if a.acknowledged_by}
              {t("tol.alerts.ackedBy")} {a.acknowledged_by}
            {:else}
              {#if data.canAcknowledge}
              <form method="POST" action="?/acknowledge">
                <input type="hidden" name="alert" value={a.id} />
                <button class="button" type="submit">{t("tol.alerts.ack")}</button>
              </form>
              {/if}
            {/if}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}
