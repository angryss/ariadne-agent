import { Moon, Sun } from 'lucide-react';
import { useLayoutEffect, useState } from 'react';

import { Button } from './ui/button';

type Theme = 'light' | 'dark';

const THEME_STORAGE_KEY = 'rynna-theme';

function initialTheme(): Theme {
  try {
    const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
    if (stored === 'light' || stored === 'dark') return stored;
  } catch {
    // Storage can be unavailable in privacy-restricted browser contexts.
  }
  return window.matchMedia?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}

export function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>(initialTheme);

  useLayoutEffect(() => {
    document.documentElement.classList.remove('light', 'dark');
    document.documentElement.classList.add(theme);
    document.documentElement.style.colorScheme = theme;
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, theme);
    } catch {
      // The selected theme still applies for the current session.
    }
  }, [theme]);

  const nextTheme = theme === 'dark' ? 'light' : 'dark';
  return (
    <Button
      aria-label={`Switch to ${nextTheme} theme`}
      className="theme-toggle"
      onClick={() => setTheme(nextTheme)}
      size="icon"
      title={`Switch to ${nextTheme} theme`}
      type="button"
      variant="outline"
    >
      {theme === 'dark' ? <Sun aria-hidden="true" /> : <Moon aria-hidden="true" />}
    </Button>
  );
}
