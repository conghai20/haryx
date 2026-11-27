// Runs the keyring unwrap off the main thread: age-encryption's scrypt is synchronous, and at
// log_n 18 it would freeze the page for the whole unlock (plan amendment A2). One message in, one
// out; the page terminates the worker afterwards, which frees scrypt's 256 MiB.
import { KeyringError, unwrapKeyring } from "./keyring";

export interface UnwrapRequest {
  body: string;
  passphrase: string;
}

export type UnwrapReply = { ok: true; secret: string } | { ok: false; code: KeyringError["code"]; message: string };

self.onmessage = async (event: MessageEvent<UnwrapRequest>) => {
  let reply: UnwrapReply;
  try {
    reply = { ok: true, secret: await unwrapKeyring(event.data.body, event.data.passphrase) };
  } catch (err) {
    reply =
      err instanceof KeyringError
        ? { ok: false, code: err.code, message: err.message }
        : { ok: false, code: "unreadable", message: "The keyring could not be opened in this browser." };
  }
  self.postMessage(reply);
};
