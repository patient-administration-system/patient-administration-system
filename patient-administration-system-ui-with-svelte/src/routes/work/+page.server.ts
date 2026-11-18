import type { Actions, PageServerLoad } from "./$types";
import { fail } from "@sveltejs/kit";
import { requireStaff } from "#lib/server/guard.js";
import {
  WORK_QUEUES,
  claimWork,
  releaseWork,
  takeNextWork,
  workItems,
  workReminders,
  type WorkQueue,
} from "#lib/server/pas.js";
import { fromPas, pageFromPas, pasFailure, signedInFetch } from "#lib/server/route.js";
import { isUuid } from "#lib/access.js";

const VIEWS = ["mine", "unassigned", "overdue", "all"] as const;
type View = (typeof VIEWS)[number];

const isQueue = (q: string | null): q is WorkQueue => !!q && (WORK_QUEUES as readonly string[]).includes(q);
const isView = (v: string | null): v is View => !!v && (VIEWS as readonly string[]).includes(v);

/** One staff work queue (TSC-1): the signed-in person's reminders, and a list to work from. */
export const load: PageServerLoad = async ({ locals, fetch, url }) => {
  const session = requireStaff(locals, "/work");
  const askedQueue = url.searchParams.get("queue");
  const queue = isQueue(askedQueue) ? askedQueue : null;
  const askedView = url.searchParams.get("view");
  const view: View = isView(askedView) ? askedView : "mine";
  const f = signedInFetch(fetch, locals);
  const filter = {
    queue,
    owner: view === "mine" ? ("me" as const) : view === "unassigned" ? ("none" as const) : null,
    overdue: view === "overdue",
  };
  const items = await pageFromPas(url, (cursor) => workItems(f, filter, cursor));
  return {
    me: session.user.sub,
    queue,
    queues: WORK_QUEUES,
    view,
    views: VIEWS,
    reminders: await fromPas(workReminders(f)),
    items: items.rows,
    cutAt: items.cutAt,
    nextHref: items.nextHref,
    firstHref: items.firstHref,
  };
};

const itemOf = async (request: Request) => String((await request.formData()).get("item") ?? "");

export const actions: Actions = {
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
