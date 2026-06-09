// my.oliv.ai login for the desktop app. We point it at the same-origin
// my.oliv.ai/recorder-auth bridge page, which after login redirects to
// olivrecorder://auth-callback?ic_token=... The Rust deep-link handler
// (auth.rs) persists the token to the OS keychain and emits `oliv-auth-changed`.
//
// Use `redirect=` (NOT `final-page=`): Login.tsx drives the post-login client
// redirect off the `redirect` query param; `final-page` is ignored on that path.
export const OLIV_LOGIN_URL =
  'https://my.oliv.ai/login?redirect=' +
  encodeURIComponent('https://my.oliv.ai/recorder-auth');
