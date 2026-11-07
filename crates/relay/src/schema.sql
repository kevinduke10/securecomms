PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA secure_delete = ON;
CREATE TABLE IF NOT EXISTS devices (
  tenant TEXT NOT NULL,
  device TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  principal TEXT NOT NULL,
  revoked INTEGER NOT NULL CHECK (revoked IN (0,1)),
  PRIMARY KEY (tenant,device)
);
CREATE TABLE IF NOT EXISTS conversations (
  tenant TEXT NOT NULL,
  id TEXT NOT NULL,
  metadata TEXT NOT NULL,
  sequence INTEGER NOT NULL,
  PRIMARY KEY (tenant,id)
);
CREATE TABLE IF NOT EXISTS events (
  tenant TEXT NOT NULL,
  conversation TEXT NOT NULL,
  sequence INTEGER NOT NULL,
  sender TEXT NOT NULL,
  message_id TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  payload TEXT,
  expires_at INTEGER NOT NULL,
  PRIMARY KEY (tenant,conversation,sequence),
  UNIQUE (tenant,conversation,sender,message_id),
  FOREIGN KEY (tenant,conversation) REFERENCES conversations(tenant,id)
);
CREATE INDEX IF NOT EXISTS events_expiry ON events(expires_at) WHERE payload IS NOT NULL;
