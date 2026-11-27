<script lang="ts">
  import type { ActionData, PageData } from "./$types";
  import { i18n, t } from "#lib/i18n.svelte.js";
  import ListCut from "#lib/ListCut.svelte";

  let { data, form }: { data: PageData; form: ActionData } = $props();
  const when = (iso: string) =>
    new Intl.DateTimeFormat(i18n.locale, { dateStyle: "medium", timeStyle: "short" }).format(new Date(iso));
  const href = (view: string, queue: string | null) => `?view=${view}${queue ? `&queue=${queue}` : ""}`;
  const owner = (o: string | null | undefined) => (!o ? t("work.owner.none") : o === data.me ? t("work.owner.you") : o);
  const remindersCount = $derived(data.reminders.overdue.length + data.reminders.due_today.length);
</script>

<svelte:head><title>{t("work.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("work.title")}</h1>
<p>{t("work.intro")}</p>

<section aria-labelledby="reminders">
  <h2 id="reminders">{t("work.reminders.title")}</h2>
  {#if remindersCount === 0}
    <p data-testid="reminders-none">{t("work.reminders.none")}</p>
  {:else}
    <p role="status" data-testid="reminders">
      {t("work.reminders.summary", {
        overdue: data.reminders.overdue.length,
        today: data.reminders.due_today.length,
        by: when(data.reminders.due_by),
      })}
    </p>
    <ul>
      {#each [...data.reminders.overdue, ...data.reminders.due_today] as r (r.id)}
        <li data-reminder={r.id}>
          {t(`work.kind.${r.kind}`)} –
          {#if r.overdue}<strong>{t("work.overdue")}</strong>{/if}
          {t("work.due")}
          {when(r.due_at)}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<nav aria-label={t("work.views")}>
  <strong>{t("work.views")}:</strong>
  {#each data.views as v (v)}
    {#if v === data.view}
      <span aria-current="page">{t(`work.view.${v}`)}</span>
    {:else}
      <a href={href(v, data.queue)}>{t(`work.view.${v}`)}</a>
    {/if}
  {/each}
</nav>
<nav aria-label={t("work.queues")}>
  <strong>{t("work.queues")}:</strong>
  {#if data.queue === null}
    <span aria-current="page">{t("work.queue.all")}</span>
  {:else}
    <a href={href(data.view, null)}>{t("work.queue.all")}</a>
  {/if}
  {#each data.queues as q (q)}
    {#if q === data.queue}
      <span aria-current="page">{t(`work.queue.${q}`)}</span>
    {:else}
      <a href={href(data.view, q)}>{t(`work.queue.${q}`)}</a>
    {/if}
  {/each}
</nav>

<section aria-labelledby="saved-views">
  <h2 id="saved-views">{t("work.saved")}</h2>
  {#if data.savedViews.length === 0}
    <p data-testid="saved-none">{t("work.saved.none")}</p>
  {:else}
    <ul data-testid="saved-views">
      {#each data.savedViews as v (v.id)}
        <li data-view={v.id}>
          {#if data.saved?.id === v.id}
            <span aria-current="page">{v.name}</span>
          {:else}
            <a href={`?saved=${v.id}`}>{v.name}</a>
          {/if}
          {#if v.created_by !== data.me}({t("work.saved.by", { name: v.created_by })}){/if}
          {#if v.shared_with_role}({t("work.saved.shared", { role: v.shared_with_role })}){/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if data.saved && data.saved.created_by === data.me}
    <!-- The view stays in the address, so the page shows it again after the action. -->
    <form method="POST" action={`?saved=${data.saved.id}&/shareView`}>
      <input type="hidden" name="saved" value={data.saved.id} />
      <label>
        {t("work.saved.role")}
        <input name="role" required maxlength="100" autocomplete="off" value={data.saved.shared_with_role ?? ""} />
      </label>
      <button class="button" type="submit">{t("work.saved.share")}</button>
    </form>
    {#if data.saved.shared_with_role}
      <form method="POST" action={`?saved=${data.saved.id}&/unshareView`}>
        <input type="hidden" name="saved" value={data.saved.id} />
        <button class="button" type="submit">{t("work.saved.unshare")}</button>
      </form>
    {/if}
    <form method="POST" action="?/deleteView">
      <input type="hidden" name="saved" value={data.saved.id} />
      <button class="button" type="submit">{t("work.saved.delete")}</button>
    </form>
  {:else if !data.saved}
    <form method="POST" action="?/saveView">
      <input type="hidden" name="view" value={data.view} />
      <input type="hidden" name="queue" value={data.queue ?? ""} />
      <label>
        {t("work.saved.name")}
        <input name="name" required maxlength="100" autocomplete="off" />
      </label>
      <button class="button" type="submit">{t("work.saved.save")}</button>
    </form>
  {/if}
</section>

<form method="POST" action="?/next">
  <label>
    {t("work.next.queue")}
    <select name="queue">
      {#each data.queues as q (q)}
        <option value={q} selected={q === data.queue}>{t(`work.queue.${q}`)}</option>
      {/each}
    </select>
  </label>
  <button class="button" type="submit">{t("work.next")}</button>
</form>

{#if form?.done}
  <p role="status">{t(form.done)}</p>
{:else if form?.messageKey}
  <p role="alert">{t(form.messageKey)}</p>
{/if}

{#if data.items.length === 0}
  <p data-testid="work-none">{t("work.none")}</p>
{:else}
  <ListCut at={data.cutAt} next={data.nextHref} first={data.firstHref} />
  <table data-work>
    <caption>{data.saved ? data.saved.name : t(`work.view.${data.view}`)}</caption>
    <thead>
      <tr>
        <th scope="col">{t("work.col.what")}</th>
        <th scope="col">{t("work.col.queue")}</th>
        <th scope="col">{t("work.col.priority")}</th>
        <th scope="col">{t("work.col.due")}</th>
        <th scope="col">{t("work.col.owner")}</th>
        <th scope="col">{t("work.col.action")}</th>
      </tr>
    </thead>
    <tbody>
      {#each data.items as w (w.id)}
        <tr data-item={w.id}>
          <th scope="row">{t(`work.kind.${w.kind}`)}</th>
          <td>{t(`work.queue.${w.queue}`)}</td>
          <td>{t(`work.priority.${w.priority}`)}</td>
          <td>
            {when(w.due_at)}
            {#if w.overdue}<br /><strong>{t("work.overdue")}</strong>{/if}
          </td>
          <td>{owner(w.owner)}</td>
          <td>
            {#if w.status === "open"}
              <form method="POST" action="?/claim">
                <input type="hidden" name="item" value={w.id} />
                <button class="button" type="submit">{t("work.claim")}</button>
              </form>
            {:else if w.owner === data.me}
              <form method="POST" action="?/release">
                <input type="hidden" name="item" value={w.id} />
                <button class="button" type="submit">{t("work.release")}</button>
              </form>
            {/if}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}
<p><small>{t("work.how")}</small></p>
