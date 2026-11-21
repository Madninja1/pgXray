"use strict";

function hideUserEditors() {
  byId("create-user-panel").hidden = true;
  byId("rename-panel").hidden = true;
  byId("user-password-panel").hidden = true;

  byId("user-form").reset();
  byId("rename-form").reset();
  byId("user-password-form").reset();

  delete byId("rename-form").dataset.username;
  delete byId("user-password-form").dataset.username;
}

function openCreateUser() {
  hideUserEditors();

  byId("create-user-panel").hidden = false;
  byId("user-form").elements.username.focus();
}

function userActionButton(
    label,
    actionName,
    username,
    className = "secondary",
) {
  const button = document.createElement("button");

  button.type = "button";
  button.textContent = label;
  button.className = className;
  button.dataset.userAction = actionName;
  button.dataset.username = username;

  return button;
}

function renderUsers(users) {
  const body = byId("user-list");

  body.replaceChildren();

  for (const user of users) {
    const row = document.createElement("tr");
    const usernameCell = document.createElement("td");
    const statusCell = document.createElement("td");
    const actionsCell = document.createElement("td");
    const actions = document.createElement("div");

    usernameCell.textContent = user.username;
    statusCell.textContent = user.current ? "Текущий" : "Пользователь";
    statusCell.classList.toggle("status-current", user.current);
    actions.className = "row-actions";

    actions.append(
        userActionButton(
            "Изменить логин",
            "rename",
            user.username,
        ),
        userActionButton(
            "Сменить пароль",
            "password",
            user.username,
        ),
    );

    const deleteButton = userActionButton(
        "Удалить",
        "delete",
        user.username,
        "danger",
    );

    deleteButton.disabled = user.current;

    actions.append(deleteButton);

    actionsCell.append(actions);
    row.append(usernameCell, statusCell, actionsCell);
    body.append(row);
  }
}

async function refreshUsers() {
  renderUsers(
      await invoke("list_users"),
  );
}

function openRename(username) {
  hideUserEditors();

  const form = byId("rename-form");

  form.dataset.username = username;
  form.elements.username.value = username;

  byId("rename-user-label").textContent = username;
  byId("rename-panel").hidden = false;

  form.elements.username.focus();
}

function openUserPassword(username) {
  hideUserEditors();

  const form = byId("user-password-form");

  form.dataset.username = username;

  byId("password-user-label").textContent = username;
  byId("user-password-panel").hidden = false;

  form.elements.password.focus();
}

function initSettingsUi() {
  byId("open-user-create").addEventListener(
      "click",
      openCreateUser,
  );

  byId("cancel-user-create").addEventListener(
      "click",
      hideUserEditors,
  );

  byId("user-form").addEventListener("submit", (event) => {
    event.preventDefault();

    const form = event.currentTarget;
    const data = new FormData(form);

    void action(async () => {
      if (data.get("password") !== data.get("confirm")) {
        throw new Error("Пароли не совпадают");
      }

      await invoke(
          "create_user",
          {
            username: data.get("username"),
            password: data.get("password"),
          },
      );

      hideUserEditors();
      await refreshUsers();

      message("Пользователь создан.");
    }).then(() => {
      clearPasswordInputs(form);
    });
  });

  byId("rename-form").addEventListener("submit", (event) => {
    event.preventDefault();

    const form = event.currentTarget;
    const originalUsername = form.dataset.username;
    const data = new FormData(form);
    const newUsername = String(
        data.get("username"),
    );

    void action(async () => {
      await invoke(
          "rename_user",
          {
            username: originalUsername,
            newUsername,
          },
      );

      if (originalUsername === currentUsername) {
        setSession({
          username: newUsername,
        });
      }

      hideUserEditors();
      await refreshUsers();

      message("Логин пользователя изменён.");
    });
  });

  byId("user-password-form").addEventListener("submit", (event) => {
    event.preventDefault();

    const form = event.currentTarget;
    const username = form.dataset.username;
    const data = new FormData(form);

    void action(async () => {
      if (data.get("password") !== data.get("confirm")) {
        throw new Error("Пароли не совпадают");
      }

      await invoke(
          "change_user_password",
          {
            username,
            password: data.get("password"),
          },
      );

      hideUserEditors();

      message(
          `Пароль пользователя ${username} изменён.`,
      );
    }).then(() => {
      clearPasswordInputs(form);
    });
  });

  byId("user-list").addEventListener("click", (event) => {
    const button = event.target.closest(
        "button[data-user-action]",
    );

    if (!button) {
      return;
    }

    const username = button.dataset.username;

    if (button.dataset.userAction === "rename") {
      openRename(username);
      return;
    }

    if (button.dataset.userAction === "password") {
      openUserPassword(username);
      return;
    }

    if (button.dataset.userAction === "delete") {
      if (
          !window.confirm(
              `Удалить пользователя ${username}?`,
          )
      ) {
        return;
      }

      void action(async () => {
        await invoke(
            "delete_user",
            {
              username,
            },
        );

        hideUserEditors();
        await refreshUsers();

        message(
            `Пользователь ${username} удалён.`,
        );
      });
    }
  });

  for (const id of [
    "cancel-rename",
    "cancel-user-password",
  ]) {
    byId(id).addEventListener(
        "click",
        hideUserEditors,
    );
  }
}
