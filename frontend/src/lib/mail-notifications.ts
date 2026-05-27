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

export function showNewMailNotification(
  mail: NewMailNotification,
  tl: (key: string) => string,
): void {
  if (!mailNotificationsEnabled()) return;

  const notification = new Notification(tl('mail_notificationTitle'), {
    body: `${mail.from || tl('mail_unknownSender')}\n${mail.subject || tl('mail_noSubject')}`,
    icon: '/icons/icon-192x192.png',
    tag: `${mail.account_id}|${mail.mailbox}|${mail.uid}`,
  });
  notification.onclick = () => {
    window.focus();
    window.location.assign(`/mail/view/?account_id=${encodeURIComponent(mail.account_id)}&mailbox=${encodeURIComponent(mail.mailbox)}&uid=${mail.uid}`);
    notification.close();
  };
}
