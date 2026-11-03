import { listCollections, scanRoots, type Collection } from "./api";

export type Route =
  | { name: "library"; collectionId?: string }
  | { name: "series"; id: string; identify?: boolean }
  | { name: "import"; paths: string[]; fromRoots?: boolean }
  | { name: "identify" }
  | { name: "reader"; chapterId: string }
  | { name: "settings" };

class AppStore {
  route = $state<Route>({ name: "library" });
  search = $state("");
  collections = $state<Collection[]>([]);
  /** Incrémenté après un changement de données : les vues rechargent. */
  revision = $state(0);
  error = $state<string | null>(null);
  /** Éléments des dossiers suivis qui ne sont pas encore dans la bibliothèque. */
  newInRoots = $state<string[]>([]);

  go(route: Route) {
    this.route = route;
    this.error = null;
  }

  async changed() {
    this.revision += 1;
    await this.loadCollections();
  }

  /** Rapide : seuls les noms de fichiers sont comparés, aucune archive n'est ouverte. */
  async scanRoots() {
    const scans = await scanRoots();
    this.newInRoots = scans.flatMap((scan) => scan.newPaths);
  }

  async loadCollections() {
    this.collections = await listCollections();
  }

  /** Exécute une action et affiche son échec au lieu de le perdre. */
  async attempt<T>(action: () => Promise<T>): Promise<T | undefined> {
    this.error = null;
    try {
      return await action();
    } catch (error) {
      this.error = String(error);
      return undefined;
    }
  }
}

export const app = new AppStore();
