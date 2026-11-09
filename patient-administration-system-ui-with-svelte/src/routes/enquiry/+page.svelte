<script lang="ts">
  import type { PageData } from "./$types";
  import { i18n, t } from "#lib/i18n.svelte.js";

  let { data }: { data: PageData } = $props();
  const when = (iso: string) => new Intl.DateTimeFormat(i18n.locale, { dateStyle: "medium" }).format(new Date(iso));
  const day = (iso: string) =>
    new Intl.DateTimeFormat(i18n.locale, { dateStyle: "medium", timeZone: "UTC" }).format(new Date(iso));
</script>

<svelte:head><title>{t("enquiry.title")} – {t("app.name")}</title></svelte:head>
<h1>{t("enquiry.title")}</h1>
<p>{t("enquiry.intro")}</p>
<form method="GET" role="search">
  <label for="patient">{t("col.patientId")}</label>
  <input id="patient" name="patient" type="text" value={data.patient} />
  <button class="button" type="submit">{t("waitlist.show")}</button>
</form>
{#if data.invalid}
  <p role="alert">{t("history.invalid")}</p>
{:else if data.enquiry}
  {@const e = data.enquiry}
  <h2>{t("enquiry.suspicions")}</h2>
  {#if e.suspected_conditions.length === 0}<p>{t("enquiry.none")}</p>{:else}
    <ul>
      {#each e.suspected_conditions as s (s.id)}
        <li>
          {t(`condition.${s.condition}`)}{s.tumour_site ? ` (${t(`tumour.${s.tumour_site}`)})` : ""}:
          {day(s.suspected_on)}, {s.days_waited}
          {t("suspicions.col.days")},
          {t(`target.${s.target_status}`, { days: s.target_days ?? 0 })}{s.stop_reason ? `, ${s.stop_reason}` : ""}
        </li>
      {/each}
    </ul>
  {/if}
  <h2>{t("enquiry.appointments")}</h2>
  {#if e.appointments.length === 0}<p>{t("enquiry.none")}</p>{:else}
    <ul>
      {#each e.appointments as a (a.id)}<li>{when(a.start_datetime)}: {t(`status.${a.status}`)}</li>{/each}
    </ul>
  {/if}
  <h2>{t("enquiry.referrals")}</h2>
  {#if e.referrals.length === 0}<p>{t("enquiry.none")}</p>{:else}
    <ul>
      {#each e.referrals as r (r.id)}<li>{when(r.received_at)}: {r.target_service}, {r.status}</li>{/each}
    </ul>
  {/if}
  <h2>{t("enquiry.waitlist")}</h2>
  {#if e.waitlist_entries.length === 0}<p>{t("enquiry.none")}</p>{:else}
    <ul>
      {#each e.waitlist_entries as w (w.id)}<li>{when(w.created_at)}: {w.target_service}, {w.status}</li>{/each}
    </ul>
  {/if}
  <h2>{t("enquiry.pathways")}</h2>
  {#if e.rtt_pathways.length === 0}<p>{t("enquiry.none")}</p>{:else}
    <ul>
      {#each e.rtt_pathways as p (p.id)}<li>{when(p.started_at)}: {p.target_service}, {p.status}</li>{/each}
    </ul>
  {/if}
  <h2>{t("enquiry.admissions")}</h2>
  {#if e.admissions.length === 0}<p>{t("enquiry.none")}</p>{:else}
    <ul>
      {#each e.admissions as a (a.id)}<li>{when(a.admitted_at)}</li>{/each}
    </ul>
  {/if}
  <h2>{t("enquiry.letters")}</h2>
  {#if e.letters.length === 0}<p>{t("enquiry.none")}</p>{:else}
    <ul>
      {#each e.letters as l (l.id)}<li>{when(l.created_at)}: {t(`channel.${l.channel}`)}, {l.status}</li>{/each}
    </ul>
  {/if}
{/if}
