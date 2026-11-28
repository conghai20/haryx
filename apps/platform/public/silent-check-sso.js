// Hands Keycloak's silent check-sso answer (code or login_required, in this page's URL) back to
// HaryX web, which opened this page in a hidden iframe. Only the same origin may read it.
parent.postMessage(location.href, location.origin);
