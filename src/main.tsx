import React from 'react';
import ReactDOM from 'react-dom/client';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import App from './App';
import Library from './flow/Library';
import { remembered, speak } from './locales';
import './styles.css';

// The Library window loads the same page; everything else is the companion.
const library = isTauri() ? getCurrentWindow().label === 'library' : new URLSearchParams(location.search).has('library');
document.documentElement.classList.toggle('is-desktop', isTauri() && !library);

// In the language last spoken, so the first paint isn't in English; Rust's word follows.
void speak(remembered()).finally(() => ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>{library ? <Library /> : <App />}</React.StrictMode>,
));
