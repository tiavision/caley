import { m } from '@tiaforge-ui/sinter/messages';

export interface NewMailNotification {
  account_id: string;
  uid: number;
  mailbox: string;
  from: string | null;
  subject: string | null;
}

export const mailNotificationsStorageKey = 'mail-notifications-enabled';

export function mailNotificationsSupported(): boolean {
  return 'Notification' in window;
}

export function mailNotificationsEnabled(): boolean {
  return mailNotificationsSupported() &&
    Notification.permission === 'granted' &&
    localStorage.getItem(mailNotificationsStorageKey) === '1';
}

export function showNewMailNotification(mail: NewMailNotification): void {
  if (!mailNotificationsEnabled()) return;

  const notification = new Notification(m.mail_notificationTitle(), {
    body: `${mail.from || m.mail_unknownSender()}\n${mail.subject || m.mail_noSubject()}`,
    icon: '/icons/icon-192x192.png',
    tag: `${mail.account_id}|${mail.mailbox}|${mail.uid}`,
  });
  notification.onclick = () => {
    window.focus();
    window.location.assign(`/mail/view/?account_id=${encodeURIComponent(mail.account_id)}&mailbox=${encodeURIComponent(mail.mailbox)}&uid=${mail.uid}`);
    notification.close();
  };
}
