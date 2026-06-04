export interface Account {
  account_id: string;
  display_name: string;
  host: string;
  port: number;
  username: string;
  use_tls: boolean;
  smtp_host: string | null;
  smtp_port: number | null;
}

export interface Identity {
  identity_id: string;
  account_id: string;
  name: string;
  email: string;
  is_default: boolean;
  sort_order: number;
}

export interface Mailbox {
  account_id: string;
  name: string;
  delimiter: string | null;
  subscribed: boolean;
}

export interface SidebarMailbox {
  mailbox: Mailbox;
  unread_count: number;
}

export interface SidebarAccountSection {
  account: Account;
  collapsed: boolean;
  unread_count: number;
  mailboxes: SidebarMailbox[];
}

export interface VirtualFolder {
  folder_id: string;
  display_name: string;
  sort_order: number;
  only_flagged: boolean;
  only_attachments: boolean;
  rules: { account_id: string | null; mailbox_pattern: string | null }[];
}

export interface Attachment {
  filename: string;
  content_type: string;
  size: number;
  content_id: string | null;
}

export interface Message {
  uid: number;
  account_id: string;
  mailbox: string;
  rowid: number;
  message_id: string | null;
  thread_id: string | null;
  from: string | null;
  to: string[];
  cc: string[];
  subject: string | null;
  sent_at: number | null;
  flags: string[];
  has_attachments: boolean;
  raw_eml: string | null;
  is_read: boolean;
  is_flagged: boolean;
  is_encrypted: boolean;
  attachments: Attachment[];
}

export interface ThreadNode {
  uid: number;
  account_id: string;
  mailbox: string;
  message_id: string | null;
  parent_message_id: string | null;
  from: string | null;
  subject: string | null;
  sent_at: number | null;
  is_read: boolean;
}

export interface ThreadTreeNode {
  message: ThreadNode;
  children: ThreadTreeNode[];
}

export type FolderKind = 'inbox' | 'sent' | 'drafts' | 'trash' | 'junk' | 'archive' | 'folder';

export type BodySegment = { text: string } | { href: string; text: string };
