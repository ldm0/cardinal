import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import './i18n/config';

const rootElement = document.getElementById('root');

if (!rootElement) {
  throw new Error('Unable to initialize application: #root element is missing.');
}

ReactDOM.createRoot(rootElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
