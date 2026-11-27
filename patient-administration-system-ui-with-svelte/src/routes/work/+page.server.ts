import type { Actions, PageServerLoad } from "./$types";
import { fail } from "@sveltejs/kit";
import { requireStaff } from "#lib/server/guard.js";
import {
  WORK_QUEUES,
  claimWork,
  deleteWorkView,
  releaseWork,
  saveWorkView,
  shareWorkView,
  takeNextWork,
  workItems,
  workReminders,
  workViewItems,
  workViews,
  type WorkFilter,
  type WorkQueue,
} from "#lib/server/pas.js";
import { fromPas, pageFromPas, pasFailure, signedInFetch } from "#lib/server/route.js";
import { isUuid } from "#lib/access.js";

const VIEWS = ["mine", "unassigned", "overdue", "all"] as const;
type View = (typeof VIEWS)[number];

const isQueue = (q: string | null): q is WorkQueue => !!q && (WORK_QUEUES as readonly string[]).includes(q);
const isView = (v: string | null): v is View => !!v && (VIEWS as readonly string[]).includes(v);

/** The filter a built-in view and queue stand for. */
const filterOf = (view: View, queue: WorkQueue | null): WorkFilter => ({
  queue,
  owner: view === "mine" ? "me" : view === "unassigned" ? "none" : null,
  overdue: view === "overdue",
});

/** One staff work queue (TSC-1): the signed-in person's reminders, and a list to work from, built in or saved
 * (TSC-7). */
export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  const session = requireStaff(locals, "/work");
  const askedQueue = url.searchParams.get("queue");
  const queue = isQueue(askedQueue) ? askedQueue : null;
  const askedView = url.searchParams.get("view");
  const view: View = isView(askedView) ? askedView : "mine";
  const f = signedInFetch(fetch, locals);
  const savedViews = await fromPas(workViews(f));
  const askedSaved = url.searchParams.get("saved");
  const saved = savedViews.find((v) => v.id === askedSaved) ?? null;
  const items = await pageFromPas(url, (cursor) =>
    saved ? workViewItems(f, saved.id, cursor) : workItems(f, filterOf(view, queue), cursor),
  );
  return {
    me: session.user.sub,
    queue,
    queues: WORK_QUEUES,
    view,
    views: VIEWS,
    savedViews,
    saved,
    reminders: await fromPas(workReminders(f)),
    items: items.rows,
    cutAt: items.cutAt,
    nextHref: items.nextHref,
    firstHref: items.firstHref,
  };
};

const viewOf = (form: FormData) => String(form.get("saved") ?? "");

const itemOf = async (request: Request) => String((await request.formData()).get("item") ?? "");

export const actions: Actions = {
  // Save what is shown (a built-in view and queue) under a name of the signed-in person's (TSC-7).
  saveView: async ({ request, locals, fetch }) => {
    requireStaff(locals, "/work");
    const form = await request.formData();
    const name = String(form.get("name") ?? "").trim();
    const view = String(form.get("view") ?? "");
    const queue = String(form.get("queue") ?? "");
    if (!name || name.length > 100 || !isView(view)) return fail(400, { id: "", messageKey: "work.error.name" });
    const failed = await pasFailure(
      () => saveWorkView(signedInFetch(fetch, locals), name, filterOf(view, isQueue(queue) ? queue : null)),
      { id: "", messageKey: "work.error.name" },
    );
    return failed ?? { id: "", done: "work.done.saved" };
  },
  shareView: async ({ request, locals, fetch }) => {
    requireStaff(locals, "/work");
    const form = await request.formData();
    const id = viewOf(form);
    const role = String(form.get("role") ?? "").trim();
    if (!isUuid(id) || !role || role.length > 100) return fail(400, { id, messageKey: "work.error" });
    const failed = await pasFailure(() => shareWorkView(signedInFetch(fetch, locals), id, role), {
      id,
      messageKey: "work.error",
    });
    return failed ?? { id, done: "work.done.shared" };
  },
  unshareView: async ({ request, locals, fetch }) => {
    requireStaff(locals, "/work");
    const id = viewOf(await request.formData());
    if (!isUuid(id)) return fail(400, { id, messageKey: "work.error" });
    const failed = await pasFailure(() => shareWorkView(signedInFetch(fetch, locals), id, null), {
      id,
      messageKey: "work.error",
    });
    return failed ?? { id, done: "work.done.unshared" };
  },
  deleteView: async ({ request, locals, fetch }) => {
    requireStaff(locals, "/work");
    const id = viewOf(await request.formData());
    if (!isUuid(id)) return fail(400, { id, messageKey: "work.error" });
    const failed = await pasFailure(() => deleteWorkView(signedInFetch(fetch, locals), id), {
      id,
      messageKey: "work.error",
    });
    return failed ?? { id, done: "work.done.deleted" };
  },
  // The person acting is always the signed-in member of staff (`X-On-Behalf-Of`), never a name in the form.
  claim: async ({ request, locals, fetch }) => {
    requireStaff(locals, "/work");
    const id = await itemOf(request);
    if (!isUuid(id)) return fail(400, { id, messageKey: "work.error" });
    const failed = await pasFailure(() => claimWork(signedInFetch(fetch, locals), id), {
      id,
      messageKey: "work.error.taken",
    });
    return failed ?? { id, done: "work.done.claimed" };
  },
  release: async ({ request, locals, fetch }) => {
    requireStaff(locals, "/work");
    const id = await itemOf(request);
    if (!isUuid(id)) return fail(400, { id, messageKey: "work.error" });
    const failed = await pasFailure(() => releaseWork(signedInFetch(fetch, locals), id), {
      id,
      messageKey: "work.error",
    });
    return failed ?? { id, done: "work.done.released" };
  },
  next: async ({ request, locals, fetch }) => {
    requireStaff(locals, "/work");
    const queue = String((await request.formData()).get("queue") ?? "");
    if (!isQueue(queue)) return fail(400, { id: "", messageKey: "work.error" });
    const failed = await pasFailure(() => takeNextWork(signedInFetch(fetch, locals), queue), {
      id: "",
      messageKey: "work.error.empty",
    });
    return failed ?? { id: "", done: "work.done.next" };
  },
};
