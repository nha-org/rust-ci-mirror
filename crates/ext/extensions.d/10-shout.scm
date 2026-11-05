;; An extractor, with no rebuild: a file copied to <data>/extensions.d/. The tests beside it run under nextest.
(define (shout text) (string-upcase text))

(register-extractor! "text/x-shout" shout)

(test! "it shouts" (lambda () (check-equal? (shout "quiet") "QUIET")))
