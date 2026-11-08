(() => {
  const USERNAME_TYPES = new Set(["text", "email", "tel", "url", ""]);
  const visible = (el) =>
    !el.disabled && !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length);
  const passwordFields = () =>
    [...document.querySelectorAll("input[type=password]")].filter(visible);
  const usernameFor = (password) => {
    const scope = password.form ? [...password.form.elements] : [...document.querySelectorAll("input")];
    const index = scope.indexOf(password);
    for (let i = index - 1; i >= 0; i--) {
      const el = scope[i];
      if (el instanceof HTMLInputElement && USERNAME_TYPES.has(el.type) && visible(el)) {
        return el;
      }
    }
    return document.querySelector("input[autocomplete~=username]");
  };
  const fieldName = (el) => (el ? el.name || el.id || "" : "");
  const setValue = (el, value) => {
    el.focus();
    el.value = value;
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    el.blur();
  };
  return {
    detect() {
      return passwordFields().length > 0 ? location.origin : "";
    },
    capture() {
      const password = passwordFields().find((el) => el.value);
      if (!password) {
        return "";
      }
      const username = usernameFor(password);
      let action = location.origin;
      if (password.form && password.form.getAttribute("action")) {
        try {
          action = new URL(password.form.action, location.href).origin;
        } catch {
          action = location.origin;
        }
      }
      return JSON.stringify({
        origin: location.origin,
        formActionOrigin: action,
        usernameField: fieldName(username),
        passwordField: fieldName(password),
        username: username ? username.value : "",
        password: password.value,
      });
    },
    fill(origin, username, password) {
      if (location.origin !== origin) {
        return false;
      }
      const field = passwordFields().find((el) => !el.readOnly);
      if (!field || field.value) {
        return false;
      }
      const user = usernameFor(field);
      if (user && user.value && user.value !== username) {
        return false;
      }
      if (user && !user.value && username) {
        setValue(user, username);
      }
      setValue(field, password);
      return true;
    },
  };
})()
