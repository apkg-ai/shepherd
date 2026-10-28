import type { ReactNode } from "react";
import { Link } from "react-router";
import type { ButtonVariant } from "./Button";
import buttonStyles from "./Button.module.css";

interface LinkButtonProps {
  to: string;
  variant?: ButtonVariant;
  children: ReactNode;
}

/** Anchors styled as buttons — avoids nesting a button inside a link. */
export function LinkButton({ to, variant = "secondary", children }: LinkButtonProps) {
  return (
    <Link to={to} className={`${buttonStyles.button} ${buttonStyles[variant]}`}>
      {children}
    </Link>
  );
}
