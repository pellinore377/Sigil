# Push contract

Push is a generic wakeup; it carries no sender, conversation, message text, ciphertext or file key. Authenticated polling and message acknowledgement remain authoritative. Provider success is not proof of recipient delivery.

Each device has one revisioned registration and coalesced job. A pending challenge proves endpoint ownership before activation. Replacement/cancellation invalidate earlier registration work. Invalid provider registrations stop delivery; transport/configuration failures retain work with bounded backoff and completion-time Retry-After.

Native registrations renew before their 30-day expiry with a fresh ownership challenge. An invalid target waits for replacement or explicit `retry_push_registration` after provider repair; repeating the same FCM token callback does not reactivate it. FCM-wide failures back off from one minute exponentially; other failures start at five seconds, with jitter and a one-hour base cap.

FCM uses HTTP v1 with short-lived OAuth credentials. UnifiedPush uses RFC 8291 Web Push encryption and RFC 8292 VAPID with a separate P-256 application-server key. Provider keys never become messaging identities. Tokens/endpoints are sensitive capabilities.

HTTPS work validates resolved addresses, TLS hostname and response bounds. Redirects/proxies are refused. Private providers require operator-configured egress exceptions; clients cannot authorize them. Source-device revocation, registration revisions and stale worker leases are rechecked before results commit.

Restore resets pending registration/delivery state; restored credentials cannot silently resume pushes. Native decryption and registration state persist separately from live messaging keys.

Configure providers with revisioned `GET/PUT /admin/v0/push`: `unified_push`, VAPID `contact`, operator egress `exceptions`, `rotate_vapid`, and `fcm.action` (`keep`, `disable`, `configure`). FCM configuration takes `credentials.project_id`, `client_email`, and PKCS#8 `private_key`; GET never returns private keys. Native clients use `/client/v0/push/providers`, `GET/PUT/DELETE /client/v0/push`, and `POST /client/v0/push/confirm`.

`GET/PUT /admin/v0/push/android` manages public Firebase `android` options (`project_id`, `application_id`, `sender_id`, `api_key`; null removes them). PUT requires `expected_revision` and `expected_push_revision`. Authenticated devices fetch `/client/v0/push/android`; provider generation changes and restore invalidate these options. Android initializes the default Firebase app from saved options, reconciles durable opt-out at startup, and requires a cold restart for project changes. It never receives service-account keys.

`sync_backend_due_online` includes the durable push lane. Platform adapters supply token/endpoint changes and push bytes to the native APIs, then schedule work using returned deadlines. Hints never bypass mailbox authentication, backoff or local decryption. FCM uses normal-priority data messages because encrypted traffic can be silent; Doze can delay delivery. Distributor selection, Android callbacks/permissions, notification construction and testing with two distributors remain platform acceptance.

References: [UnifiedPush](https://unifiedpush.org/developers/intro/), [FCM HTTP v1](https://firebase.google.com/docs/cloud-messaging/send/v1-api), [FCM failures](https://firebase.google.com/docs/cloud-messaging/error-codes), [FCM priority](https://firebase.google.com/docs/cloud-messaging/customize-messages/setting-message-priority), [RFC 8291](https://www.rfc-editor.org/rfc/rfc8291.html), [RFC 8292](https://www.rfc-editor.org/rfc/rfc8292.html). Dependency notices are in `licenses/`; provider framing tests live beside the implementation.
