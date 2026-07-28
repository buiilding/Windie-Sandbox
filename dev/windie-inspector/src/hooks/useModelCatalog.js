import { useCallback, useEffect, useMemo, useState } from "react";
import {
  fetchModelParameters,
  listModels,
} from "@/lib/windieApi";

/** Owns model discovery, parameter loading, and their transient request state. */
export function useModelCatalog({ gatewayRunning, activeModelId }) {
  const [models, setModels] = useState([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState(null);
  const [modelParametersById, setModelParametersById] = useState({});

  const refreshModels = useCallback(async () => {
    setModelsLoading(true);
    try {
      const nextModels = await listModels();
      setModels(nextModels);
      setModelsError(null);
      return nextModels;
    } catch (error) {
      setModels([]);
      setModelsError(error.message);
      throw error;
    } finally {
      setModelsLoading(false);
    }
  }, []);

  const loadModelParameters = useCallback(
    async (modelId) => {
      if (!modelId || !gatewayRunning || modelsLoading || modelsError) return null;
      if (!models.some((model) => model.id === modelId)) return null;
      const existing = modelParametersById[modelId];
      if (existing?.status === "ready") return existing.data;
      if (existing?.status === "loading" || existing?.status === "error") return null;

      setModelParametersById((previous) => ({
        ...previous,
        [modelId]: {
          status: "loading",
          data: previous[modelId]?.data || null,
          error: null,
        },
      }));
      try {
        const data = await fetchModelParameters(modelId);
        setModelParametersById((previous) => ({
          ...previous,
          [modelId]: { status: "ready", data, error: null },
        }));
        return data;
      } catch (error) {
        setModelParametersById((previous) => ({
          ...previous,
          [modelId]: { status: "error", data: null, error: error.message },
        }));
        return null;
      }
    },
    [gatewayRunning, modelParametersById, models, modelsError, modelsLoading]
  );

  useEffect(() => {
    refreshModels().catch(() => {});
  }, [refreshModels]);

  const activeCatalogModel = useMemo(
    () => models.find((model) => model.id === activeModelId) || null,
    [activeModelId, models]
  );

  useEffect(() => {
    if (activeCatalogModel) loadModelParameters(activeModelId);
  }, [activeCatalogModel, activeModelId, loadModelParameters]);

  return {
    models,
    modelsLoading,
    modelsError,
    modelParametersById,
    activeCatalogModel,
    activeModelParameters: modelParametersById[activeModelId] || null,
    refreshModels,
    loadModelParameters,
  };
}
