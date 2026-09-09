Local library runtime shared by native FFI and the desktop client.

- Opens and migrates the existing database, retains profile IDs, and owns polling/download tasks.
- API dispatch uses the shared router in process without a socket.
- Each request checks current profile authorization; media access also enforces subscriptions and path confinement.
- Library shutdown invalidates existing sessions and closes the database before deletion.
