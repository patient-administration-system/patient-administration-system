import type { RequestHandler } from "./$types";
import { error, redirect } from "@sveltejs/kit";
import { THEMES, THEME_COOKIE, safeBack } from "#lib/theme.js";

// The theme menu's form posts here (it works with no script). The choice is kept for a year and
// survives sign-out; `device` clears it so the browser's setting decides again.
export const POST: RequestHandler = async ({ request, cookies, url }) => {
  const form = await request.formData();
  const theme = String(form.get("theme") ?? "");
  if (!(THEMES as readonly string[]).includes(theme)) error(400, "unknown theme");
  if (theme === "device") {
    cookies.delete(THEME_COOKIE, { path: "/" });
  } else {
    cookies.set(THEME_COOKIE, theme, {
      path: "/",
      maxAge: 60 * 60 * 24 * 365,
      sameSite: "lax",
      httpOnly: true,
      secure: url.protocol === "https:",
    });
  }
  redirect(303, safeBack(String(form.get("back") ?? "")));
};
