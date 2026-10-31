import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";

// Avant le premier affichage : les boutons de fenêtre de macOS occupent la zone de
// titre, et le thème mémorisé évite un éclair de la mauvaise couleur.
document.documentElement.dataset.platform = /Mac/.test(navigator.platform) ? "mac" : "other";
try {
  const theme = localStorage.getItem("komago-theme");
  if (theme === "light" || theme === "dark") document.documentElement.dataset.theme = theme;
} catch {
  // Le stockage peut être indisponible : le thème s'appliquera au chargement des réglages.
}

export default mount(App, { target: document.getElementById("app")! });
