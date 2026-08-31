import { useEffect, useState } from 'react';

import { Input } from './ui/input';

export interface TypeaheadProps {
  disabled?: boolean;
  id: string;
  onChange: (value: string) => void;
  options: readonly string[];
  value: string;
}

export function Typeahead({ disabled, id, onChange, options, value }: TypeaheadProps) {
  const [query, setQuery] = useState(value);
  const [open, setOpen] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [active, setActive] = useState<string | null>(value || null);

  useEffect(() => {
    setQuery(value);
    setDirty(false);
    setActive(value || null);
  }, [value]);

  const matches = matchingOptions(options, query, !dirty);

  function cancelEditing() {
    setQuery(value);
    setOpen(false);
    setDirty(false);
    setActive(value || null);
  }

  function select(next: string) {
    onChange(next);
    setQuery(next);
    setOpen(false);
    setDirty(false);
    setActive(next);
  }

  return (
    <div className="provider-typeahead">
      <Input
        aria-activedescendant={
          open && active && matches.includes(active)
            ? `${id}-option-${matches.indexOf(active)}`
            : undefined
        }
        aria-autocomplete="list"
        aria-controls={`${id}-options`}
        aria-expanded={open}
        autoComplete="off"
        disabled={disabled}
        id={id}
        onBlur={cancelEditing}
        onChange={(event) => {
          const nextQuery = event.target.value;
          setQuery(nextQuery);
          setOpen(true);
          setDirty(true);
          setActive(matchingOptions(options, nextQuery, false)[0] ?? null);
        }}
        onFocus={(event) => {
          event.currentTarget.select();
          setOpen(true);
          const nextMatches = matchingOptions(options, query, !dirty);
          setActive(nextMatches.includes(value) ? value : (nextMatches[0] ?? null));
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            cancelEditing();
            return;
          }
          const nextMatches = matchingOptions(options, query, !dirty);
          if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            if (nextMatches.length === 0) return;
            event.preventDefault();
            setOpen(true);
            const currentIndex = active ? nextMatches.indexOf(active) : -1;
            const offset = event.key === 'ArrowDown' ? 1 : -1;
            const nextIndex =
              currentIndex === -1
                ? event.key === 'ArrowDown'
                  ? 0
                  : nextMatches.length - 1
                : (currentIndex + offset + nextMatches.length) % nextMatches.length;
            setActive(nextMatches[nextIndex] ?? null);
            return;
          }
          if (event.key !== 'Enter') return;
          const match = (active && nextMatches.includes(active) ? active : nextMatches[0]) ?? null;
          if (!match) return;
          event.preventDefault();
          select(match);
        }}
        role="combobox"
        value={query}
      />
      {open ? (
        <div className="provider-type-options" id={`${id}-options`} role="listbox">
          {matches.map((option, index) => (
            <button
              aria-selected={option === active}
              id={`${id}-option-${index}`}
              key={option}
              onClick={() => select(option)}
              onMouseDown={(event) => event.preventDefault()}
              onMouseMove={() => setActive(option)}
              role="option"
              tabIndex={-1}
              type="button"
            >
              {option}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function matchingOptions(options: readonly string[], query: string, showAll: boolean): string[] {
  return [...options]
    .sort((left, right) => left.localeCompare(right))
    .filter((option) => showAll || option.toLowerCase().includes(query.toLowerCase()));
}
