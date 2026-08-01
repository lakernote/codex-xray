import { Search, X } from "lucide-react";

type Props = {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  ariaLabel: string;
  clearLabel: string;
  className?: string;
};

export default function SearchField({
  value,
  onChange,
  placeholder,
  ariaLabel,
  clearLabel,
  className = "",
}: Props) {
  return (
    <div className={`app-search-field ${className}`.trim()}>
      <Search aria-hidden="true" />
      <input
        type="search"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        aria-label={ariaLabel}
      />
      {value ? (
        <button
          type="button"
          onClick={() => onChange("")}
          aria-label={clearLabel}
        >
          <X aria-hidden="true" />
        </button>
      ) : null}
    </div>
  );
}
