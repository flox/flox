--- filter-links.lua – remove all links from a doc
---
--- This filter removes all href links from a document,
--- and replaces them with strong/bold text.
---
--- Short term mitigation for https://github.com/jgm/pandoc/issues/9458

-- Links have been included with pandoc lua filter sine pandoc 2.0.0
PANDOC_VERSION:must_be_at_least '2.0'

return {
  { Link = function (elem)

      local link = pandoc.Inlines({"<", pandoc.Underline(elem.target), ">"})

      if elem.content ~= nil then
        local elems = pandoc.Inlines({})
        elems:extend({pandoc.Strong(elem.content)})
        if elem.target:find("^%w+://") ~= nil then
          elems:extend({" "})
          elems:extend(link)
        end

        return elems
      end

      return link

    end
   }
}
