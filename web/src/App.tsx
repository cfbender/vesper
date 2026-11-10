import { Layout } from "./components/Layout";
import { Notice, PageTitle } from "./components/ui";
import { LiveProvider } from "./data";
import { EditorPage } from "./pages/Editor";
import { IncidentPage, IncidentsPage } from "./pages/Incidents";
import { MachinePage } from "./pages/Machine";
import { Overview } from "./pages/Overview";
import { ServicePage } from "./pages/Service";
import { SettingsPage } from "./pages/Settings";
import { Link, paths, useLocationKey, useRoute } from "./router";

function Page() {
  const route = useRoute();
  const key = useLocationKey();
  switch (route.name) {
    case "overview":
      return <Overview />;
    case "machine":
      return <MachinePage key={key} machineId={route.machine} />;
    case "service":
      return <ServicePage key={key} machineId={route.machine} serviceId={route.service} />;
    case "new-service":
      return <EditorPage key={key} machineId={null} serviceId={null} presetMachine={route.machine} />;
    case "edit-service":
      return <EditorPage key={key} machineId={route.machine} serviceId={route.service} />;
    case "incidents":
      return <IncidentsPage />;
    case "incident":
      return <IncidentPage key={key} id={route.id} />;
    case "settings":
      return <SettingsPage />;
    default:
      return (
        <>
          <PageTitle title="Page not found" />
          <Notice>
            <p>
              <Link to={paths.overview()}>Back to the overview</Link>
            </p>
          </Notice>
        </>
      );
  }
}

export function App() {
  return (
    <LiveProvider>
      <Layout>
        <Page />
      </Layout>
    </LiveProvider>
  );
}
