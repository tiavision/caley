// The script of <auth-return>: the navigation after the sign-in on the login page. (Already signed in, the
// server redirects `/` itself; on the Vite dev server, which cannot, this goes to the calendar.)
import { AuthReturnComponent } from './auth-return.html';

export class AuthReturn extends AuthReturnComponent {
  connected() {
    location.replace(this.props.to || '/calendar/');
  }
}
