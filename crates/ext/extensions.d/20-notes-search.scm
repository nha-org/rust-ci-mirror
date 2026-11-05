;; One extension adds a search bar, an MCP tool and a rerank hook (@nha, 2026-11-05: « create plugins too maybe »).

;; The search bar: a UI tree of catalog elements, data that the renderers draw (sections 10 and 12).
(register-ui! "search"
  (hash "root" "notes-search"
        "elements"
        (hash "notes-search" (hash "type" "stack" "children" (list "bar" "hint"))
              "bar" (hash "type" "search-bar" "props" (hash "placeholder" "Search your notes"))
              "hint" (hash "type" "text" "props" (hash "text" "A title match comes first.")))))

;; The MCP tool: the words a search would match on.
(define (lowercase-words text) (map string-downcase (split-whitespace text)))

(register-tool! "query_terms" "The lowercase words of a search query."
  (hash "type" "object"
        "properties" (hash "query" (hash "type" "string"))
        "required" (list "query"))
  (lambda (arguments) (lowercase-words (hash-ref arguments 'query))))

;; The rerank hook: a hit whose title holds the query goes first, the others keep their order.
(define (in-title? query hit)
  (string-contains? (string-downcase (hash-ref hit 'title)) (string-downcase query)))

(define (title-first query hits)
  (append (filter (lambda (hit) (in-title? query hit)) hits)
          (filter (lambda (hit) (not (in-title? query hit))) hits)))

(register-rerank! "title-first" title-first)

(test! "the words are lowercase"
  (lambda () (check-equal? (lowercase-words "Rust  PODS") (list "rust" "pods"))))

(test! "a title match goes first, the others keep their order"
  (lambda ()
    (define (hit id title) (hash 'id id 'title title))
    (define hits (list (hit "1" "other") (hit "2" "A RUST note") (hit "3" "more")))
    (check-equal? (map (lambda (h) (hash-ref h 'id)) (title-first "rust" hits))
                  (list "2" "1" "3"))))
