Notification state and API-error presentation for the UI.

- `ToastQueue` and `ToastHandle` collect messages consumed by the toast container.
- Foreground validation can remain inline; background failures surface through `ToastDecision`.
- Connection failures and expired authentication are decisions for the caller, rather than ordinary toasts.
