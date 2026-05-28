import type { ButtonHTMLAttributes, ReactNode } from "react";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  readonly children: ReactNode;
  readonly variant?: "default" | "primary";
}

export function Button({ children, className, variant = "default", type = "button", ...props }: ButtonProps) {
  const variantClass = variant === "primary" ? "button-primary" : "";
  return (
    <button className={["button", variantClass, className].filter(Boolean).join(" ")} type={type} {...props}>
      {children}
    </button>
  );
}
