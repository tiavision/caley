// The script of <auth-return>: just signed in on the login page, ask `/` again; the server now redirects to
// the page wanted or last used (backend/src/landing.rs).
import { AuthReturnComponent } from './auth-return.html';

export class AuthReturn extends AuthReturnComponent {
  connected() {
    location.replace('/');
  }
}
