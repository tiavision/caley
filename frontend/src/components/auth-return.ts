// The script of <auth-return>: the navigation after sign-in (src/lib/return-url.ts).
import { redirectToReturnUrl } from '../lib/return-url.ts';
import { AuthReturnComponent } from './auth-return.html';

export class AuthReturn extends AuthReturnComponent {
  connected() {
    redirectToReturnUrl();
  }
}
