import { type FormEvent, useState } from "react";
import { useNavigate } from "react-router";
import { useTranslation } from "react-i18next";
import { useCreateBrowserSession } from "../../api/generated/identity/identity";
import { isShepherdError } from "../../api/problem";
import { Button } from "../../components/Button";
import { FormField } from "../../components/FormField";
import inputStyles from "../../components/Input.module.css";
import { PageHeader } from "../../components/PageHeader";
import styles from "./LoginScreen.module.css";

export function LoginScreen() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [token, setToken] = useState("");
  const [fieldError, setFieldError] = useState<string>();

  const login = useCreateBrowserSession({
    mutation: {
      meta: { silent: true },
      onSuccess: () => {
        setToken("");
        void navigate("/", { replace: true });
      },
      onError: (error) => {
        setToken("");
        if (isShepherdError(error) && error.status === 401) {
          setFieldError(t("auth.login.invalidToken"));
        } else if (isShepherdError(error) && error.status === 403) {
          setFieldError(t("auth.login.forbidden"));
        } else {
          setFieldError(t("common.error.unexpected"));
        }
      },
    },
  });

  const handleSubmit = (event: FormEvent) => {
    event.preventDefault();
    setFieldError(undefined);
    login.mutate({ data: { owner_token: token.trim() } });
  };

  return (
    <div className={styles.container}>
      <div className={styles.card}>
        <PageHeader title={t("auth.login.title")} description={t("auth.login.description")} />
        <form className={styles.form} onSubmit={handleSubmit} noValidate>
          <FormField
            label={t("auth.login.tokenLabel")}
            error={fieldError}
            hint={t("auth.login.tokenHint")}
          >
            {(props) => (
              <input
                {...props}
                type="password"
                className={inputStyles.mono}
                value={token}
                onChange={(e) => setToken(e.target.value)}
                placeholder={t("auth.login.tokenPlaceholder")}
                autoComplete="current-password"
                required
              />
            )}
          </FormField>
          <Button type="submit" variant="primary" busy={login.isPending} disabled={!token.trim()}>
            {t("auth.login.submit")}
          </Button>
        </form>
      </div>
    </div>
  );
}
