import { defineCollection, z } from "astro:content";
import { docsLoader } from "@astrojs/starlight/loaders";
import { docsSchema } from "@astrojs/starlight/schema";

const guideSchema = z.object({
  guide: z
    .object({
      eyebrow: z.string().optional(),
      displayTitle: z.string().optional(),
      accentBar: z.boolean().optional(),
    })
    .optional(),
});

export const collections = {
  docs: defineCollection({
    loader: docsLoader(),
    schema: docsSchema({ extend: guideSchema }),
  }),
};
